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
  SessionSnapshot{id:format!("codex:{}",root.id),client:"codex".into(),title:root.title.chars().take(80).collect(),state:state.into(),label:label.clone(),message:label,updated_at:now,created_at:root.started}
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
  rows.push(Row{id,title,parent,state,updated,started:started.map(|v|v.max(0) as u64).unwrap_or(0)});
 }
 Ok(aggregate(&rows,now))
}
pub fn start(){std::thread::spawn(||loop{
 // Sessions live in the local sqlite even when the process name is not exactly
 // "codex", so the list must not wait on presence before the first read.
 let active=crate::presence::get_active_clients().iter().any(|c|c=="codex");
 if let Some(home)=dirs::home_dir(){
  let root=home.join(".codex");
  if root.join("state_5.sqlite").is_file(){if let Ok(rows)=read(&root){if let Ok(mut cache)=CACHE.lock(){*cache=Some((Instant::now(),rows));}}}
 }
 std::thread::sleep(Duration::from_millis(if active {500} else {2000}));
});}
pub fn sessions()->Vec<SessionSnapshot>{CACHE.lock().ok().and_then(|c|c.as_ref().filter(|(at,_)|at.elapsed()<Duration::from_secs(5)).map(|(_,r)|r.clone())).unwrap_or_default()}

const MAX_PROMPTS: usize = 16;
const MAX_PROMPT_CHARS: usize = 180;

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

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
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

fn is_user_event(value: &serde_json::Value) -> bool {
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
        return false;
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
        return false;
    }
    if payload_kind == "user_message"
        || payload_kind == "user"
        || kind == "user_message"
        || kind == "user"
    {
        return true;
    }
    kind == "response_item" && role == "user" && (payload_kind == "message" || payload_kind.is_empty())
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

fn prompt_from_line(line: &str) -> Option<String> {
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
    if !is_user_event(&value) {
        return None;
    }
    let collapsed = collapse_ws(&extract_user_text(&value)?);
    if collapsed.is_empty() {
        return None;
    }
    Some(truncate_chars(&collapsed, MAX_PROMPT_CHARS))
}

#[cfg(test)]
fn parse_user_prompts(text: &str) -> Vec<String> {
    let mut kept = Vec::new();
    for line in text.lines() {
        if let Some(prompt) = prompt_from_line(line) {
            if kept.last() == Some(&prompt) {
                continue;
            }
            kept.push(prompt);
            if kept.len() > MAX_PROMPTS {
                kept.remove(0);
            }
        }
    }
    kept
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

fn read_prompts_file(path: &Path) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|err| format!("读取会话日志失败：{err}"))?;
    let reader = BufReader::new(file);
    let mut kept = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|err| format!("读取会话日志失败：{err}"))?;
        if let Some(prompt) = prompt_from_line(&line) {
            if kept.last() == Some(&prompt) {
                continue;
            }
            kept.push(prompt);
            if kept.len() > MAX_PROMPTS {
                kept.remove(0);
            }
        }
    }
    Ok(kept)
}

fn user_prompts_in(root: &Path, session_id: &str) -> Result<Vec<String>, String> {
    let thread_id = session_id.strip_prefix("codex:").unwrap_or(session_id);
    if !valid_thread_id(thread_id) {
        return Err("无效的会话 ID".to_string());
    }
    let Some(path) = resolve_rollout(root, thread_id)? else {
        return Ok(Vec::new());
    };
    read_prompts_file(&path)
}

pub fn user_prompts(session_id: &str) -> Result<Vec<String>, String> {
    let home = dirs::home_dir().ok_or("无法定位用户目录")?;
    user_prompts_in(&home.join(".codex"), session_id)
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
 fn user_prompts_keep_only_the_latest_ones() {
  let mut text = String::new();
  for index in 0..20 {
   text.push_str(&format!(
    "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"user_message\",\"message\":\"问题 {index}\"}}}}\n"
   ));
  }
  let prompts = parse_user_prompts(&text);
  assert_eq!(prompts.len(), 16);
  assert_eq!(prompts[0], "问题 4");
  assert_eq!(prompts[15], "问题 19");
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
