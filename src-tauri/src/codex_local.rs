//! Local read-only task lifecycle adapter. Does not require Hooks or parse answers as completion.
use crate::sessions::SessionSnapshot;
use rusqlite::{Connection,OpenFlags};
use std::{collections::{HashMap,HashSet},fs::File,io::{BufRead,BufReader,Read,Seek,SeekFrom},path::{Path,PathBuf},sync::Mutex,time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
static CACHE:Mutex<Option<(Instant,Vec<SessionSnapshot>)>>=Mutex::new(None);
#[derive(Clone,Debug,PartialEq)]
struct Lifecycle {state:&'static str,turn:String,started:String}
fn lifecycle(text:&str)->Lifecycle {
 advance(text,Lifecycle{state:"idle",turn:String::new(),started:String::new()})
}
fn advance(text:&str,mut result:Lifecycle)->Lifecycle {
 for line in text.lines(){
  let Ok(v)=serde_json::from_str::<serde_json::Value>(line) else {continue};
  if v["type"]!="event_msg"{continue;}
  let p=&v["payload"];let turn=p["turn_id"].as_str().unwrap_or("");
  match p["type"].as_str().unwrap_or("") {
   "task_started"=>{result=Lifecycle{state:"working",turn:turn.into(),started:v["timestamp"].as_str().unwrap_or("").into()};},
   "task_complete"|"turn_aborted" if result.turn.is_empty()||turn==result.turn=>{result.state="completed";result.turn=turn.into();},
   // Item/tool completion and assistant text are NOT a completed task.
   _=>{}
  }
 }
 result
}
fn read_lifecycle(path:&Path)->std::io::Result<Lifecycle>{
 type Entry=(u64,SystemTime,u64,Lifecycle);
 static FILES:std::sync::LazyLock<Mutex<HashMap<std::path::PathBuf,Entry>>>=std::sync::LazyLock::new(||Mutex::new(HashMap::new()));
 let mut f=File::open(path)?;let meta=f.metadata()?;let size=meta.len();let modified=meta.modified()?;
 let mut cache=FILES.lock().unwrap();
 if let Some((len,time,_,state))=cache.get(path){if *len==size&&*time==modified{return Ok(state.clone());}}
 let previous=cache.get(path).filter(|(len,_,offset,_)|size>*len&&size.saturating_sub(*offset)<=8*1024*1024);
 let (mut start,initial)=previous.map(|(_,_,offset,state)|(*offset,state.clone())).unwrap_or((size.saturating_sub(8*1024*1024),lifecycle("")));
 let incremental=previous.is_some();f.seek(SeekFrom::Start(start))?;
 let mut bytes=Vec::new();f.take(8*1024*1024).read_to_end(&mut bytes)?;
 if !incremental&&start>0{if let Some(end)=bytes.iter().position(|b|*b==b'\n'){bytes.drain(..=end);start+=end as u64+1;}}
 let consumed=bytes.iter().rposition(|b|*b==b'\n').map(|p|p+1).unwrap_or(0);
 let state=advance(&String::from_utf8_lossy(&bytes[..consumed]),initial);
 if cache.len()>512{cache.clear();}
 cache.insert(path.to_path_buf(),(size,modified,start+consumed as u64,state.clone()));
 Ok(state)
}
struct Row{id:String,title:String,parent:Option<String>,state:&'static str,updated:u64,started:u64}
fn aggregate(rows:&[Row],now:u64)->Vec<SessionSnapshot>{
 let parents:HashMap<&str,&str>=rows.iter().filter_map(|r|r.parent.as_deref().map(|p|(r.id.as_str(),p))).collect();
 rows.iter().filter(|r|r.parent.is_none()).map(|root|{
  let mut working=0;let mut unknown=0;
  for child in rows.iter().filter(|c|c.parent.is_some()&&c.updated>=root.started){
   let mut id=child.id.as_str();let mut seen=HashSet::new();let mut belongs=false;
   while let Some(parent)=parents.get(id){if !seen.insert(id){break;}if *parent==root.id{belongs=true;break;}id=parent;}
   if belongs {if child.state=="working"{working+=1;}else if child.state=="idle"{unknown+=1;}}
  }
  let state=if root.state=="working"||working>0{"working"}else if unknown>0{"idle"}else{root.state};
  let label=if working>0{format!("运行中 · {} 个子代理",working)}else{match state{"working"=>"运行中".into(),"completed"=>"已停止".into(),_=>"状态待确认".into()}};
  SessionSnapshot{id:format!("codex:{}",root.id),client:"codex".into(),title:root.title.clone(),state:state.into(),label:label.clone(),message:label,updated_at:now,created_at:root.started}
 }).collect()
}
fn read(home:&Path)->Result<Vec<SessionSnapshot>,Box<dyn std::error::Error>>{
 let db=Connection::open_with_flags(home.join("state_5.sqlite"),OpenFlags::SQLITE_OPEN_READ_ONLY|OpenFlags::SQLITE_OPEN_NO_MUTEX)?;db.busy_timeout(Duration::from_millis(100))?;
 let now=SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
 let mut q=db.prepare("SELECT id,title,source,rollout_path,updated_at FROM threads WHERE archived=0 AND updated_at>=?1 ORDER BY updated_at DESC LIMIT 256")?;
 let data=q.query_map([now.saturating_sub(86400) as i64],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?.max(0) as u64)))?;
 let mut rows=vec![];
 for item in data{let(id,title,source,path,updated)=item?;
  let source=serde_json::from_str::<serde_json::Value>(&source).unwrap_or_default();
  let parent=source["subagent"]["thread_spawn"]["parent_thread_id"].as_str().map(str::to_string);
  let life=read_lifecycle(Path::new(&path)).unwrap_or(Lifecycle{state:"idle",turn:String::new(),started:String::new()});
  let started:Option<i64>=db.query_row("SELECT unixepoch(?1)",[&life.started],|r|r.get(0))?;
  let state=if life.state=="working"&&now.saturating_sub(updated)>1800{"idle"}else{life.state};
  let Some(title)=session_title(&title,Path::new(&path)).or_else(|| parent.is_some().then(|| fallback_title(&title))) else { continue; };
  rows.push(Row{id,title,parent,state,updated,started:started.map(|v|v.max(0) as u64).unwrap_or(0)});
 }
 Ok(aggregate(&rows,now))
}
pub fn start(){std::thread::spawn(||{
 let mut first=true;
 loop{
  // Sessions live in the local sqlite even when the process name is not exactly
  // "codex", so the list must not wait on presence before the first read.
  // This thread is the only place that opens state_5.sqlite or rollout jsonl.
  let began=Instant::now();
  if first { crate::log_line("codex: sqlite read begin"); }
  let active=crate::presence::get_active_clients().iter().any(|c|c=="codex");
  if let Some(home)=dirs::home_dir(){
   let root=home.join(".codex");
   if root.join("state_5.sqlite").is_file(){if let Ok(rows)=read(&root){if let Ok(mut cache)=CACHE.lock(){*cache=Some((Instant::now(),rows));}}}
  }
  let elapsed=began.elapsed();
  if first || elapsed>Duration::from_millis(200) {
   crate::log_line(&format!("codex: sqlite read done in {}ms", elapsed.as_millis()));
  }
  first=false;
  std::thread::sleep(Duration::from_millis(if active {500} else {2000}));
 }
});}
pub fn sessions()->Vec<SessionSnapshot>{CACHE.lock().ok().and_then(|c|c.as_ref().filter(|(at,_)|at.elapsed()<Duration::from_secs(5)).map(|(_,r)|r.clone())).unwrap_or_default()}

const MAX_PROMPTS: usize = 500;

fn valid_thread_id(id: &str) -> bool {
    (8..=128).contains(&id.len())
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !out.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

fn texts_from(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(text) => vec![text.clone()],
        serde_json::Value::Array(items) => items.iter().flat_map(texts_from).collect(),
        serde_json::Value::Object(map) => {
            if map.get("role").and_then(|role| role.as_str()) == Some("assistant") {
                return Vec::new();
            }
            let kind = map.get("type").and_then(|kind| kind.as_str()).unwrap_or("");
            if matches!(
                kind,
                "output_text" | "reasoning" | "tool_use" | "tool_result" | "input_image" | "image"
            ) {
                return Vec::new();
            }
            if let Some(text) = map.get("text").and_then(|text| text.as_str()) {
                return vec![text.to_string()];
            }
            if let Some(message) = map.get("message") {
                let texts = texts_from(message);
                if !texts.is_empty() {
                    return texts;
                }
            }
            if let Some(content) = map.get("content") {
                return texts_from(content);
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// `Some(true)` is an event_msg `user_message` (or a plain `user` event): text the
/// user typed. `Some(false)` is a `response_item` role=user, which Codex also uses
/// for AGENTS.md, environment, and permission injections.
fn classify_user_event(value: &serde_json::Value) -> Option<bool> {
    let kind = value.get("type").and_then(|kind| kind.as_str()).unwrap_or("");
    let payload = value.get("payload");
    let payload_kind = payload
        .and_then(|payload| payload.get("type"))
        .and_then(|kind| kind.as_str())
        .unwrap_or("");
    let role = payload
        .and_then(|payload| payload.get("role"))
        .and_then(|role| role.as_str())
        .unwrap_or("");
    if matches!(kind, "agent_message" | "assistant")
        || matches!(role, "assistant" | "system" | "developer")
    {
        return None;
    }
    if matches!(
        payload_kind,
        "agent_message"
            | "agent_reasoning"
            | "assistant_message"
            | "task_started"
            | "task_complete"
            | "turn_aborted"
            | "token_count"
    ) {
        return None;
    }
    if payload_kind == "user_message"
        || payload_kind == "user"
        || kind == "user_message"
        || kind == "user"
    {
        return Some(true);
    }
    if kind == "response_item" && role == "user" && (payload_kind == "message" || payload_kind.is_empty())
    {
        return Some(false);
    }
    None
}

fn is_system_tag(name: &str) -> bool {
    if name.contains('_') || name.contains('-') {
        return true;
    }
    if name.len() >= 2 && name.chars().all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit()) {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.starts_with("permission")
        || matches!(
            lower.as_str(),
            "skill" | "skills" | "image" | "images" | "attachment" | "attachments"
        )
}

/// Leading `<tag ...>` is always a wrapper. Embedded blocks are removed when the
/// tag is snake_case, shouty, or one of the short Codex wrappers.
fn is_wrapper_tag(name: &str, leading: bool) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    first.is_ascii_alphabetic()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        && (leading || is_system_tag(name))
}

fn starts_injected(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with("# AGENTS.md instructions") || parse_open_tag(text).is_some()
}

fn is_image_placeholder(line: &str) -> bool {
    let lower = line.trim().to_ascii_lowercase();
    lower == "[image]"
        || lower.starts_with("[image ")
        || lower.starts_with("[image]")
        || lower == "<image>"
        || lower == "(image)"
        || lower.contains("image omitted")
        || lower.starts_with("![")
}

/// Byte length of an open tag at the start of `text`, its name, and whether it
/// is self-closing. The caller decides if that name is a wrapper.
fn parse_open_tag(text: &str) -> Option<(usize, String, bool)> {
    let bytes = text.as_bytes();
    if bytes.first() != Some(&b'<') {
        return None;
    }
    let mut index = 1;
    if matches!(bytes.get(index), Some(b'/' | b'!' | b'?')) {
        return None;
    }
    let name_start = index;
    while index < bytes.len()
        && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_' || bytes[index] == b'-')
    {
        index += 1;
    }
    if index == name_start {
        return None;
    }
    let name = text[name_start..index].to_string();
    if !name.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    while index < bytes.len() && bytes[index] != b'>' {
        index += 1;
    }
    if index >= bytes.len() {
        return None;
    }
    let self_closing = index > 0 && bytes[index - 1] == b'/';
    Some((index + 1, name, self_closing))
}

fn find_injected_span(text: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;
    while search_from < text.len() {
        let tail = &text[search_from..];
        let tag_at = tail.find('<');
        let agents_at = tail.find("# AGENTS.md instructions");
        let rel = match (tag_at, agents_at) {
            (Some(tag), Some(agents)) => tag.min(agents),
            (Some(tag), None) => tag,
            (None, Some(agents)) => agents,
            (None, None) => return None,
        };
        let at = search_from + rel;
        if text[at..].starts_with("# AGENTS.md instructions") {
            let line_end = text[at..]
                .find('\n')
                .map(|newline| at + newline + 1)
                .unwrap_or(text.len());
            // A same-line `<INSTRUCTIONS>` block must stay so the text after it survives.
            let end = text[at..line_end]
                .find('<')
                .map(|bracket| at + bracket)
                .unwrap_or(line_end);
            if end > at {
                return Some((at, end));
            }
        }
        if let Some((open_len, name, self_closing)) = parse_open_tag(&text[at..]) {
            let leading = text[..at].chars().all(char::is_whitespace);
            if !is_wrapper_tag(&name, leading) {
                search_from = at + 1;
                continue;
            }
            if self_closing {
                return Some((at, at + open_len));
            }
            let close = format!("</{name}>");
            let after = at + open_len;
            if let Some(rel_close) = text[after..].find(&close) {
                return Some((at, after + rel_close + close.len()));
            }
            // An unclosed leading wrapper is the whole injection. An unclosed
            // tag later in the text only drops the open tag, not the question.
            if leading {
                return Some((at, text.len()));
            }
            return Some((at, at + open_len));
        }
        search_from = at + 1;
    }
    None
}

fn markdown_heading_body(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('#')?.trim_start_matches('#');
    let body = rest.trim();
    if body.is_empty() { None } else { Some(body) }
}

fn is_ide_context_heading(line: &str) -> bool {
    markdown_heading_body(line)
        .is_some_and(|body| body.to_ascii_lowercase().starts_with("context from my ide"))
}

/// `## My request:` / `## My request for Codex:` and the text that follows the colon.
fn take_my_request(line: &str) -> Option<&str> {
    let body = markdown_heading_body(line)?;
    let lower = body.to_ascii_lowercase();
    let marker_len = if lower.starts_with("my request for codex") {
        "my request for codex".len()
    } else if lower.starts_with("my request") {
        "my request".len()
    } else {
        return None;
    };
    let after = &lower[marker_len..];
    if !(after.is_empty()
        || after.starts_with(':')
        || after.starts_with('：')
        || after.starts_with(' ')
        || after.starts_with('\t'))
    {
        return None;
    }
    let rest = body.get(marker_len..)?.trim_start();
    Some(rest.trim_start_matches([':', '：']).trim_start())
}

/// Drop a leading `# Context from my IDE setup` section and a `## My request:`
/// heading. Text after the request heading stays; a context section after it is
/// discarded through the end of the message.
fn strip_markdown_wrappers(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let request_at = lines.iter().position(|line| take_my_request(line).is_some());
    let mut kept: Vec<&str> = Vec::new();
    let start = if let Some(at) = request_at {
        if let Some(rest) = take_my_request(lines[at]) {
            if !rest.is_empty() {
                kept.push(rest);
            }
        }
        at + 1
    } else {
        0
    };
    for line in lines.iter().skip(start) {
        if is_ide_context_heading(line) {
            break;
        }
        kept.push(*line);
    }
    if request_at.is_none()
        && kept
            .iter()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| is_ide_context_heading(line))
    {
        return String::new();
    }
    kept.join("\n")
}

fn identical_markdown_link(text: &str, start: usize) -> Option<(&str, usize)> {
    if !text[start..].starts_with('[') {
        return None;
    }
    let after = start + 1;
    let label_end = text[after..].find("](")? + after;
    let label = text[after..label_end].trim();
    let url_start = label_end + 2;
    let url_end = text[url_start..].find(')')? + url_start;
    let url = text[url_start..url_end].trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return None;
    }
    if label != url && label.trim_end_matches('/') != url.trim_end_matches('/') {
        return None;
    }
    Some((url, url_end + 1))
}

/// `[https://host/path](https://host/path)` becomes the URL once.
fn dedupe_markdown_links(text: &str) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < text.len() {
        if text.as_bytes()[index] == b'[' {
            if let Some((url, next)) = identical_markdown_link(text, index) {
                out.push_str(url);
                index = next;
                continue;
            }
        }
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn collapse_repeated_urls_line(line: &str) -> String {
    let mut out = String::new();
    let mut previous_url = String::new();
    let mut first = true;
    for token in line.split_whitespace() {
        let bare = token.trim_matches(|ch: char| {
            matches!(ch, ',' | '.' | ';' | ':' | ')' | '(' | ']' | '[' | '。' | '，' | '；')
        });
        let is_url = bare.starts_with("https://") || bare.starts_with("http://");
        if is_url && bare == previous_url {
            continue;
        }
        if !first {
            out.push(' ');
        }
        first = false;
        out.push_str(token);
        if is_url {
            previous_url = bare.to_string();
        } else {
            previous_url.clear();
        }
    }
    out
}

fn clean_user_text(raw: &str) -> Option<String> {
    let mut text = raw.to_string();
    for _ in 0..64 {
        let Some((start, end)) = find_injected_span(&text) else {
            break;
        };
        if start >= end || end > text.len() {
            break;
        }
        text.replace_range(start..end, "\n");
    }
    text = strip_markdown_wrappers(&text);
    text = dedupe_markdown_links(&text);
    let kept = text
        .lines()
        .filter_map(|line| {
            let collapsed = collapse_repeated_urls_line(line);
            let trimmed = collapsed.trim();
            if trimmed.is_empty() || is_image_placeholder(trimmed) {
                None
            } else {
                Some(collapse_ws(trimmed))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    if kept.is_empty() || starts_injected(&kept) {
        None
    } else {
        Some(kept)
    }
}

fn fallback_title(title: &str) -> String {
    clean_user_text(title).unwrap_or_else(|| "Codex 会话".to_string())
}

fn extract_user_text(value: &serde_json::Value) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    for key in ["message", "text", "content"] {
        if let Some(field) = payload.get(key) {
            let parts = texts_from(field);
            let joined = parts
                .into_iter()
                .map(|part| part.trim().to_string())
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !joined.trim().is_empty() {
                return Some(joined);
            }
        }
    }
    None
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct UserPrompt {
    pub text: String,
    /// Unix seconds. `0` when the rollout line has no timestamp.
    pub at: i64,
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u64;
    Some(era as i64 * 146097 + doe as i64 - 719468)
}

fn unix_from_rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let year: i32 = text.get(0..4)?.parse().ok()?;
    let month: u32 = text.get(5..7)?.parse().ok()?;
    let day: u32 = text.get(8..10)?.parse().ok()?;
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    if bytes.get(10) != Some(&b'T') && bytes.get(10) != Some(&b' ') {
        return None;
    }
    let hour: u32 = text.get(11..13)?.parse().ok()?;
    let minute: u32 = text.get(14..16)?.parse().ok()?;
    let second: u32 = text.get(17..19)?.parse().ok()?;
    if bytes.get(13) != Some(&b':') || bytes.get(16) != Some(&b':') {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = text.get(19..).unwrap_or("");
    if let Some(stripped) = rest.strip_prefix('.') {
        let digits = stripped
            .find(|ch: char| !ch.is_ascii_digit())
            .unwrap_or(stripped.len());
        rest = stripped.get(digits..).unwrap_or("");
    }
    let offset = if rest.is_empty() || rest.starts_with('Z') || rest.starts_with('z') {
        0
    } else if rest.starts_with('+') || rest.starts_with('-') {
        let sign = if rest.starts_with('-') { -1 } else { 1 };
        let hour_off: i64 = rest.get(1..3)?.parse().ok()?;
        let minute_off: i64 = if rest.len() >= 6 && rest.as_bytes().get(3) == Some(&b':') {
            rest.get(4..6)?.parse().ok()?
        } else if rest.len() >= 5 {
            rest.get(3..5)?.parse().ok()?
        } else {
            0
        };
        sign * (hour_off * 3600 + minute_off * 60)
    } else {
        return None;
    };
    let days = days_from_civil(year, month, day)?;
    Some(days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64 - offset)
}

fn normalize_unix(raw: i64) -> i64 {
    if raw.abs() > 10_000_000_000 {
        raw / 1000
    } else {
        raw
    }
}

fn unix_from_value(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(number) => {
            let raw = number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value as i64))?;
            Some(normalize_unix(raw))
        }
        serde_json::Value::String(text) => {
            let text = text.trim();
            if let Ok(raw) = text.parse::<i64>() {
                return Some(normalize_unix(raw));
            }
            unix_from_rfc3339(text)
        }
        _ => None,
    }
}

fn event_unix(value: &serde_json::Value) -> i64 {
    for key in ["timestamp", "ts"] {
        if let Some(found) = value.get(key).and_then(unix_from_value) {
            return found;
        }
        if let Some(found) = value
            .get("payload")
            .and_then(|payload| payload.get(key))
            .and_then(unix_from_value)
        {
            return found;
        }
    }
    0
}

fn prompt_hit_from_line(line: &str) -> Option<UserPrompt> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if line.len() > 200_000
        && !line.contains("user_message")
        && !line.contains("\"type\":\"user\"")
        && !line.contains("\"role\":\"user\"")
    {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    // Both user_message and response_item are candidates. File order decides
    // which one is shown; injected blocks are removed inside clean_user_text.
    classify_user_event(&value)?;
    Some(UserPrompt {
        text: clean_user_text(&extract_user_text(&value)?)?,
        at: event_unix(&value),
    })
}

fn prompt_from_line(line: &str) -> Option<String> {
    prompt_hit_from_line(line).map(|hit| hit.text)
}

fn push_hit(bucket: &mut Vec<UserPrompt>, hit: UserPrompt) {
    // Collapse only the copy Codex writes twice in a row. The same words typed
    // again after another prompt stay, in their original place.
    if bucket.last().map(|item| item.text.as_str()) == Some(hit.text.as_str()) {
        return;
    }
    bucket.push(hit);
    if bucket.len() > MAX_PROMPTS {
        bucket.remove(0);
    }
}

fn prompt_hits_from_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<UserPrompt> {
    let mut kept = Vec::new();
    for line in lines {
        if let Some(hit) = prompt_hit_from_line(line) {
            push_hit(&mut kept, hit);
        }
    }
    kept
}

fn prompts_from_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    prompt_hits_from_lines(lines)
        .into_iter()
        .map(|hit| hit.text)
        .collect()
}

/// `Ok(None)` means the rollout was read and has no real typed prompt, so the
/// session is hidden. `Err` means the file could not be read; the sqlite title
/// is the fallback.
fn session_title(sqlite_title: &str, rollout: &Path) -> Option<String> {
    match first_real_prompt_in_file(rollout) {
        Ok(Some(text)) => Some(text),
        Ok(None) => None,
        Err(()) => Some(fallback_title(sqlite_title)),
    }
}

fn first_real_prompt_in_file(path: &Path) -> Result<Option<String>, ()> {
    let file = File::open(path).map_err(|_| ())?;
    let reader = BufReader::new(file);
    let mut scanned = 0usize;
    for line in reader.lines() {
        let line = line.map_err(|_| ())?;
        scanned = scanned.saturating_add(line.len());
        if let Some(text) = prompt_from_line(&line) {
            return Ok(Some(text));
        }
        if scanned > 8 * 1024 * 1024 {
            break;
        }
    }
    Ok(None)
}

#[cfg(test)]
fn title_from_sources(sqlite_title: &str, rollout_text: &str) -> Option<String> {
    if rollout_text.trim().is_empty() {
        return Some(fallback_title(sqlite_title));
    }
    prompts_from_lines(rollout_text.lines()).into_iter().next()
}

#[cfg(test)]
fn parse_user_prompts(text: &str) -> Vec<String> {
    prompts_from_lines(text.lines())
}

#[cfg(test)]
fn parse_user_prompt_hits(text: &str) -> Vec<UserPrompt> {
    prompt_hits_from_lines(text.lines())
}

fn path_under(root: &Path, file: &Path) -> bool {
    let Ok(root) = root.canonicalize() else {
        return false;
    };
    let Ok(file) = file.canonicalize() else {
        return false;
    };
    file.starts_with(root)
}

fn lookup_rollout(db_path: &Path, thread_id: &str) -> Result<Option<PathBuf>, String> {
    if !db_path.is_file() {
        return Ok(None);
    }
    let db = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| format!("读取 Codex 数据库失败：{err}"))?;
    let _ = db.busy_timeout(Duration::from_millis(200));
    match db.query_row(
        "SELECT rollout_path FROM threads WHERE id = ?1",
        [thread_id],
        |row| row.get::<_, String>(0),
    ) {
        Ok(path) => Ok(Some(PathBuf::from(path))),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(err) => Err(format!("读取 Codex 会话索引失败：{err}")),
    }
}

fn find_rollout(root: &Path, thread_id: &str) -> Option<PathBuf> {
    let mut stack = vec![(root.join("sessions"), 0u8)];
    let mut seen = 0u32;
    while let Some((dir, depth)) = stack.pop() {
        if depth > 5 || seen > 4000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 4000 {
                break;
            }
            let path = entry.path();
            if path.is_dir() {
                stack.push((path, depth + 1));
                continue;
            }
            let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
            if name.contains(thread_id) && name.contains("rollout") && name.ends_with(".jsonl") {
                return Some(path);
            }
        }
    }
    None
}

fn resolve_rollout(root: &Path, thread_id: &str) -> Result<Option<PathBuf>, String> {
    if let Some(path) = lookup_rollout(&root.join("state_5.sqlite"), thread_id)? {
        if path.is_file() {
            if !path_under(root, &path) {
                return Err("rollout 路径不在 Codex 目录内".to_string());
            }
            return Ok(Some(path));
        }
    }
    if let Some(path) = find_rollout(root, thread_id) {
        if !path_under(root, &path) {
            return Err("rollout 路径不在 Codex 目录内".to_string());
        }
        return Ok(Some(path));
    }
    Ok(None)
}

fn read_prompts_file(path: &Path) -> Result<Vec<UserPrompt>, String> {
    let file = File::open(path).map_err(|err| format!("读取会话日志失败：{err}"))?;
    let reader = BufReader::new(file);
    let mut kept = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|err| format!("读取会话日志失败：{err}"))?;
        if let Some(hit) = prompt_hit_from_line(&line) {
            push_hit(&mut kept, hit);
        }
    }
    Ok(kept)
}

fn entries_in(root: &Path, session_id: &str) -> Result<Vec<UserPrompt>, String> {
    let thread_id = session_id.strip_prefix("codex:").unwrap_or(session_id);
    if !valid_thread_id(thread_id) {
        return Err("无效的会话 ID".to_string());
    }
    let Some(path) = resolve_rollout(root, thread_id)? else {
        return Ok(Vec::new());
    };
    read_prompts_file(&path)
}

#[cfg(test)]
fn user_prompts_in(root: &Path, session_id: &str) -> Result<Vec<String>, String> {
    Ok(entries_in(root, session_id)?
        .into_iter()
        .map(|item| item.text)
        .collect())
}

pub fn user_prompt_entries(session_id: &str) -> Result<Vec<UserPrompt>, String> {
    let home = dirs::home_dir().ok_or("无法定位用户目录")?;
    entries_in(&home.join(".codex"), session_id)
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn only_matching_task_end_finishes(){
  let start=r#"{"type":"event_msg","payload":{"type":"task_started","turn_id":"a"}}"#;
  let item=r#"{"type":"event_msg","payload":{"type":"item_completed","turn_id":"a"}}"#;
  let other=r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"child"}}"#;
  assert_eq!(lifecycle(&format!("{start}\n{item}\n{other}")).state,"working");
  let end=r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"a"}}"#;
  assert_eq!(lifecycle(&format!("{start}\n{end}\n{item}")).state,"completed");
 }
 #[test]fn running_grandchild_overrides_parent_completion(){
  let r=|id:&str,parent:Option<&str>,state|Row{id:id.into(),parent:parent.map(str::to_string),title:id.into(),state,updated:100,started:90};
  let rows=vec![r("root",None,"completed"),r("child",Some("root"),"completed"),r("grandchild",Some("child"),"working")];
  assert_eq!(aggregate(&rows,110)[0].state,"working");
 }
 #[test]fn child_completion_does_not_finish_parent_and_unknown_is_not_red(){
  let root=Row{id:"r".into(),parent:None,title:"r".into(),state:"working",updated:100,started:90};
  let child=Row{id:"c".into(),parent:Some("r".into()),title:"c".into(),state:"completed",updated:100,started:90};
  let mut rows=vec![root,child];assert_eq!(aggregate(&rows,110)[0].state,"working");
  rows[0].state="completed";rows[1].state="idle";assert_eq!(aggregate(&rows,110)[0].state,"idle");
  rows[1].state="completed";assert_eq!(aggregate(&rows,110)[0].state,"completed");
 }
 #[test]fn live_metadata(){if let Some(h)=dirs::home_dir(){let p=h.join(".codex");if p.join("state_5.sqlite").exists(){let rows=read(&p).unwrap();println!("Codex states: {:?}",rows.iter().map(|r|(&r.id,&r.state)).collect::<Vec<_>>());assert!(!rows.is_empty());}}}
 #[test]fn partial_log_append_and_rotation(){
  use std::io::Write;
  let path=std::env::temp_dir().join(format!("codex-lifecycle-{}-{}.jsonl",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()));
  std::fs::write(&path,"{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"a\"}}\n").unwrap();
  assert_eq!(read_lifecycle(&path).unwrap().state,"working");
  let mut file=std::fs::OpenOptions::new().append(true).open(&path).unwrap();
  file.write_all(b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",").unwrap();file.flush().unwrap();
  assert_eq!(read_lifecycle(&path).unwrap().state,"working");
  file.write_all(b"\"turn_id\":\"a\"}}\n").unwrap();file.flush().unwrap();
  assert_eq!(read_lifecycle(&path).unwrap().state,"completed");drop(file);
  std::fs::write(&path,b"\n").unwrap();assert_eq!(read_lifecycle(&path).unwrap().state,"idle");
  std::fs::remove_file(path).unwrap();
 }
 #[test]fn historical_child_events_do_not_finish_parent(){
  use std::io::{BufRead,BufReader};
  let Some(home)=dirs::home_dir() else{return;};let path=home.join(".codex/state_5.sqlite");if !path.exists(){return;}
  let db=Connection::open_with_flags(path,OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
  let child=db.query_row("SELECT id,source,rollout_path FROM threads WHERE source LIKE '%parent_thread_id%' ORDER BY updated_at DESC LIMIT 1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)));
  let Ok((child_id,source,child_path))=child else{return;};
  let source:serde_json::Value=serde_json::from_str(&source).unwrap();let parent=source["subagent"]["thread_spawn"]["parent_thread_id"].as_str().unwrap();
  let parent_path:String=db.query_row("SELECT rollout_path FROM threads WHERE id=?1",[parent],|r|r.get(0)).unwrap();
  let mut events=vec![];
  for (which,path) in [(0,parent_path),(1,child_path)]{for line in BufReader::new(File::open(path).unwrap()).lines().map_while(Result::ok){
   let Ok(v)=serde_json::from_str::<serde_json::Value>(&line)else{continue;};
   if v["type"]=="event_msg"&&matches!(v["payload"]["type"].as_str(),Some("task_started"|"task_complete"|"turn_aborted")){
    let sanitized=serde_json::json!({"type":"event_msg","timestamp":v["timestamp"],"payload":{"type":v["payload"]["type"],"turn_id":v["payload"]["turn_id"]}});
    events.push((v["timestamp"].as_str().unwrap_or("").to_string(),which,sanitized.to_string()));
   }
  }}
  events.sort_by(|a,b|a.0.cmp(&b.0));let mut states=[lifecycle(""),lifecycle("")];let mut verified=0;
  for (_,which,event) in events{states[which]=advance(&event,states[which].clone());
   let rows=vec![Row{id:parent.into(),title:"parent".into(),parent:None,state:states[0].state,started:0,updated:1},Row{id:child_id.clone(),title:"child".into(),parent:Some(parent.into()),state:states[1].state,started:0,updated:1}];
   if states[0].state=="working"||states[1].state=="working"{assert_eq!(aggregate(&rows,1)[0].state,"working");if which==1{verified+=1;}}
  }
  assert!(verified>0,"No historical active child transitions exercised");println!("Verified {verified} historical child transitions without premature red");
 }
 #[test]
 fn user_prompts_keep_questions_and_drop_replies() {
  let text = r#"
{"type":"event_msg","payload":{"type":"user_message","message":"帮我看会话列表"}}
{"type":"event_msg","payload":{"type":"agent_message","message":"这是很长的 AI 回复，用户不该看到"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"更长的回复"}]}}
{"type":"event_msg","payload":{"type":"task_started","turn_id":"a"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"第二条提问"},{"type":"output_text","text":"不该出现"}]}}
{"type":"event_msg","payload":{"type":"user_message","message":"   "}}
{"type":"user","message":"第三条提问"}
{"type":"event_msg","payload":{"type":"user_message","message":"同样的话"}}
{"type":"event_msg","payload":{"type":"user_message","message":"同样的话"}}
"#;
  assert_eq!(
   parse_user_prompts(text),
   vec!["帮我看会话列表".to_string(), "第二条提问".to_string(), "第三条提问".to_string(), "同样的话".to_string()]
  );
 }
 #[test]
 fn user_prompts_keep_file_order_and_later_repeats() {
  let text = r#"
{"type":"event_msg","payload":{"type":"user_message","message":"你好"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"你好"}]}}
{"type":"event_msg","payload":{"type":"user_message","message":"中间还有一句"}}
{"type":"event_msg","payload":{"type":"user_message","message":"你好"}}
"#;
  assert_eq!(
   parse_user_prompts(text),
   vec!["你好".to_string(), "中间还有一句".to_string(), "你好".to_string()]
  );
 }
 #[test]
 fn user_prompts_drop_injected_rollout_blocks() {
  let text = r##"
{"type":"event_msg","payload":{"type":"user_message","message":"现在可以把这个文件夹目录改为这个设计产品geo服务的海外独立站了"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for D:\\3Dauto\\nantao-backup\n<INSTRUCTIONS>\n# 工作区约定 (D:\\3Dauto) 本文件对本工作区下所有会话生效。\n</INSTRUCTIONS>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n<cwd>D:\\3Dauto\\nantao-backup</cwd>\n</environment_context>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<user_instructions>\n不要把这段系统说明显示出来\n</user_instructions>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<permissions instructions>\nallow\n</permissions>"}]}}
{"type":"event_msg","payload":{"type":"user_message","message":"<turn_aborted>\n用户中断了这一轮\n</turn_aborted>"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<skill name=\"geo\">\nwrapper\n</skill>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<attachment name=\"shot.png\">\n[Image]\n</attachment>"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"这是 AI 回复"}]}}
"##;
  assert_eq!(
   parse_user_prompts(text),
   vec!["现在可以把这个文件夹目录改为这个设计产品geo服务的海外独立站了".to_string()]
  );
 }
 #[test]
 fn response_item_strips_injected_blocks_and_keeps_typed_text() {
  let text = r##"
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for D:\\3Dauto\\nantao-backup\n<INSTRUCTIONS>\n# 工作区约定\n</INSTRUCTIONS>\n现在可以把这个文件夹目录改为海外独立站\n<environment_context>\n<cwd>D:\\3Dauto</cwd>\n</environment_context>\n<permissions>\nallow all\n</permissions>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<user_instructions>\nhidden\n</user_instructions>\n<turn_aborted></turn_aborted>\n[Image]\n请只保留这句真正的提问"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<skill>\n不要显示技能包装\n</skill>"}]}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for D:\\foo <INSTRUCTIONS>rules</INSTRUCTIONS> 同一行里的真实提问"}]}}
"##;
  assert_eq!(
   parse_user_prompts(text),
   vec![
    "现在可以把这个文件夹目录改为海外独立站".to_string(),
    "请只保留这句真正的提问".to_string(),
    "同一行里的真实提问".to_string()
   ]
  );
 }
 #[test]
 fn session_title_uses_the_cleaned_first_user_message() {
  let rollout = r##"
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for D:\\3Dauto\\nantao-backup\n<INSTRUCTIONS>\n工作区约定\n</INSTRUCTIONS>"}]}}
{"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>\n<cwd>D:\\3Dauto</cwd>\n</environment_context>\n现在可以把这个文件夹目录改为这个设计产品geo服务的海外独立站了"}}
{"type":"event_msg","payload":{"type":"user_message","message":"第二条才是后续提问"}}
"##;
  let sqlite = "# AGENTS.md instructions for D:\\3Dauto\\nantao-backup <INSTRUCTIONS> 工作区约定";
  assert_eq!(
   title_from_sources(sqlite, rollout),
   Some("现在可以把这个文件夹目录改为这个设计产品geo服务的海外独立站了".to_string())
  );
  assert_eq!(title_from_sources(sqlite, ""), Some("Codex 会话".to_string()));
  assert_eq!(
   title_from_sources("sqlite 里的短标题", ""),
   Some("sqlite 里的短标题".to_string())
  );
 }
 #[test]
 fn user_prompts_drop_generic_xml_wrappers() {
  let injected = r##"
{"type":"event_msg","payload":{"type":"user_message","message":"<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>"}}
{"type":"event_msg","payload":{"type":"user_message","message":"<recommended_plugins> Here is a list of plugins that are available but not installed..."}}
"##;
  assert!(parse_user_prompts(injected).is_empty());
  assert_eq!(title_from_sources("线程标题", injected), None);
  let mixed = r##"
{"type":"event_msg","payload":{"type":"user_message","message":"<external_codex_apps_open_page>{\"page_id\":null}</external_codex_apps_open_page>"}}
{"type":"event_msg","payload":{"type":"user_message","message":"<recommended_plugins> Here is a list of plugins that are available but not installed...</recommended_plugins>\n帮我改海外独立站，并比较 a < b"}}
{"type":"event_msg","payload":{"type":"user_message","message":"请用 <div>hello</div> 排版，不要装插件"}}
"##;
  assert_eq!(
   parse_user_prompts(mixed),
   vec![
    "帮我改海外独立站，并比较 a < b".to_string(),
    "请用 <div>hello</div> 排版，不要装插件".to_string()
   ]
  );
  assert_eq!(
   title_from_sources("线程标题", mixed),
   Some("帮我改海外独立站，并比较 a < b".to_string())
  );
 }
 #[test]
 fn user_prompts_strip_ide_request_prefixes() {
  let text = r###"
{"type":"event_msg","payload":{"type":"user_message","message":"# Context from my IDE setup:\n## Active file: src/main.ts\n文件内容不该出现"}}
{"type":"event_msg","payload":{"type":"user_message","message":"## My request:\n帮我改海外独立站的首页"}}
{"type":"event_msg","payload":{"type":"user_message","message":"# Context from my IDE setup:\n\n## Active file: src/main.ts\n\n## My request for Codex:\n前面的上下文丢掉"}}
{"type":"event_msg","payload":{"type":"user_message","message":"## My request:\n请保留这句 [https://example.com/a/very/long/path](https://example.com/a/very/long/path)\n# Context from my IDE setup:\n## Active file: src/main.ts\n不要显示这段 IDE 上下文\n[https://example.com/secret](https://example.com/secret)"}}
{"type":"event_msg","payload":{"type":"user_message","message":"## My request: 同一行的请求"}}
{"type":"event_msg","payload":{"type":"user_message","message":"## My request: 第一行\n第二行"}}
{"type":"event_msg","payload":{"type":"user_message","message":"## My requested changes stay"}}
{"type":"event_msg","payload":{"type":"user_message","message":"普通提问，不要删掉"}}
"###;
  assert_eq!(
   parse_user_prompts(text),
   vec![
    "帮我改海外独立站的首页".to_string(),
    "前面的上下文丢掉".to_string(),
    "请保留这句 https://example.com/a/very/long/path".to_string(),
    "同一行的请求".to_string(),
    "第一行\n第二行".to_string(),
    "## My requested changes stay".to_string(),
    "普通提问，不要删掉".to_string()
   ]
  );
  assert_eq!(
   title_from_sources("线程标题", text),
   Some("帮我改海外独立站的首页".to_string())
  );
  let timed = r###"
{"timestamp":"2026-10-22T07:31:00Z","type":"event_msg","payload":{"type":"user_message","message":"## My request:\n带时间的提问"}}
{"timestamp":"2026-10-22T15:31:00+08:00","type":"event_msg","payload":{"type":"user_message","message":"另一时区同一时刻"}}
{"timestamp":1792654260000,"type":"event_msg","payload":{"type":"user_message","message":"毫秒时间戳"}}
"###;
  let hits = parse_user_prompt_hits(timed);
  assert_eq!(
   hits.iter().map(|hit| hit.text.as_str()).collect::<Vec<_>>(),
   vec!["带时间的提问", "另一时区同一时刻", "毫秒时间戳"]
  );
  assert!(hits.iter().all(|hit| hit.at == 1_792_654_260));
 }
 #[test]
 fn user_prompts_keep_the_full_question() {
  let long = "问".repeat(400);
  let text = format!(
   "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"{long}\"}}}}\n"
  );
  assert_eq!(parse_user_prompts(&text), vec![long]);
 }
 #[test]
 fn user_prompts_keep_only_the_latest_ones() {
  let mut text = String::new();
  for index in 0..MAX_PROMPTS + 4 {
   text.push_str(&format!(
    "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"问题 {index}\"}}}}\n"
   ));
  }
  let prompts = parse_user_prompts(&text);
  assert_eq!(prompts.len(), MAX_PROMPTS);
  assert_eq!(prompts[0], "问题 4");
  assert_eq!(prompts[MAX_PROMPTS - 1], format!("问题 {}", MAX_PROMPTS + 3));
 }
 #[test]
 fn invalid_thread_ids_are_rejected() {
  let root = std::env::temp_dir();
  assert!(user_prompts_in(&root, "codex:../secrets").is_err());
  assert!(user_prompts_in(&root, "short").is_err());
 }
 #[test]
 fn reads_rollout_prompts_without_writing_the_database() {
  let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
  let root = std::env::temp_dir().join(format!("codex-prompts-{}-{nanos}", std::process::id()));
  let sessions = root.join("sessions");
  std::fs::create_dir_all(&sessions).unwrap();
  let rollout = sessions.join("rollout-thread-1.jsonl");
  std::fs::write(
   &rollout,
   "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"帮我看右上角\"}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\",\"message\":\"隐藏回复\"}}\n",
  ).unwrap();
  let db_path = root.join("state_5.sqlite");
  let db = Connection::open(&db_path).unwrap();
  db.execute_batch("CREATE TABLE threads (id TEXT, rollout_path TEXT);").unwrap();
  db.execute(
   "INSERT INTO threads (id, rollout_path) VALUES (?1, ?2)",
   ["thread-1", rollout.to_str().unwrap()],
  ).unwrap();
  drop(db);
  let before = std::fs::read(&db_path).unwrap();
  let prompts = user_prompts_in(&root, "codex:thread-1").unwrap();
  assert_eq!(prompts, vec!["帮我看右上角".to_string()]);
  assert_eq!(std::fs::read(&db_path).unwrap(), before);
  let _ = std::fs::remove_dir_all(&root);
 }
 #[test]
 fn rollout_outside_codex_dir_is_refused() {
  let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
  let root = std::env::temp_dir().join(format!("codex-prompts-out-{}-{nanos}", std::process::id()));
  std::fs::create_dir_all(&root).unwrap();
  let outside = std::env::temp_dir().join(format!("codex-prompts-secret-{nanos}.jsonl"));
  std::fs::write(
   &outside,
   "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"secret question\"}}\n",
  ).unwrap();
  let db_path = root.join("state_5.sqlite");
  let db = Connection::open(&db_path).unwrap();
  db.execute_batch("CREATE TABLE threads (id TEXT, rollout_path TEXT);").unwrap();
  db.execute(
   "INSERT INTO threads (id, rollout_path) VALUES (?1, ?2)",
   ["thread-9", outside.to_str().unwrap()],
  ).unwrap();
  drop(db);
  let error = user_prompts_in(&root, "codex:thread-9").unwrap_err();
  assert!(error.contains("Codex"));
  assert!(!error.contains("secret"));
  let _ = std::fs::remove_file(&outside);
  let _ = std::fs::remove_dir_all(&root);
 }
 #[test]
 fn filename_fallback_finds_rollout_without_a_db_row() {
  let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
  let root = std::env::temp_dir().join(format!("codex-prompts-scan-{}-{nanos}", std::process::id()));
  let day = root.join("sessions").join("2026").join("10").join("22");
  std::fs::create_dir_all(&day).unwrap();
  let rollout = day.join("rollout-2026-10-22T00-00-00-thread-abc.jsonl");
  std::fs::write(
   &rollout,
   "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"只靠文件名\"}}\n",
  ).unwrap();
  let prompts = user_prompts_in(&root, "codex:thread-abc").unwrap();
  assert_eq!(prompts, vec!["只靠文件名".to_string()]);
  let _ = std::fs::remove_dir_all(&root);
 }
}
