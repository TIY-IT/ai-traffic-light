use crate::sessions::SessionSnapshot;
use rusqlite::{Connection, OpenFlags};
use std::{io::{BufRead, BufReader}, process::{Command, Stdio}, sync::Mutex, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
static CACHE: Mutex<Option<(Instant, Vec<SessionSnapshot>)>> = Mutex::new(None);
static LIVE: Mutex<Option<(Instant, Vec<SessionSnapshot>)>> = Mutex::new(None);

// Live CDP signal: the SOLO renderer tracks streaming turns in this window-global collector.
fn decode_live(text: &str, now: u64) -> Vec<SessionSnapshot> {
    let Ok(rows) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let row = if rows.is_array() { match rows.get(0) { Some(row) => row, None => return Vec::new() } } else { &rows };
    if !row["connected"].as_bool().unwrap_or(false) { return Vec::new(); }
    let streams = row["streams"].as_i64().unwrap_or(0);
    let (state, label) = if streams > 0 { ("working", "运行中 · 当前会话") } else { ("idle", "暂无会话 · 已连接") };
    vec![SessionSnapshot {id:"trae-live".into(),client:"trae".into(),title:"TRAE 当前会话".into(),state:state.into(),label:label.into(),message:label.into(),created_at:now,updated_at:now}]
}

fn classify(status: i64) -> (&'static str, &'static str) {
    match status {
        1 | 2 | 3 | 9 | 10 => ("working", "运行中"),
        8 => ("waiting", "等待处理"),
        4 | 5 => ("completed", "已结束"),
        6 => ("error", "异常"),
        _ => ("idle", "暂无状态 / 待确认"),
    }
}

// Each top-level key is a SOLO mode ("work"/"code"); every mode carrying a session yields one snapshot.
fn parse_modes(text: &str, now: u64) -> Vec<SessionSnapshot> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let Some(modes) = value.as_object() else { return Vec::new() };
    modes.iter().filter_map(|(mode, value)| {
        let session = value.get("session")?.as_object()?;
        let id = session.get("sessionId").and_then(|v| v.as_str())?;
        if id.is_empty() { return None; }
        let updated = session.get("updateAt").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as u64 / 1000;
        let created = session.get("createdAt").and_then(|v| v.as_i64()).map(|v| v.max(0) as u64 / 1000).unwrap_or(updated);
        let (state, label) = classify(session.get("status").and_then(|v| v.as_i64()).unwrap_or(0));
        let age = now.saturating_sub(updated);
        let expired = if state == "working" || state == "error" { age > 1800 } else { age > 86_400 };
        if expired { return None; }
        let workspace = session.get("workspacePath").and_then(|v| v.as_str()).filter(|v| !v.is_empty())
            .or_else(|| session.get("source").and_then(|s| s.get("local_folder")).and_then(|v| v.as_str())).unwrap_or("");
        let project = std::path::Path::new(workspace).file_name().and_then(|p| p.to_str()).unwrap_or("");
        let name = session.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let title = if name.is_empty() { "TRAE 会话".to_string() } else if project.is_empty() { name.to_string() } else { format!("{project} · {name}") };
        Some(SessionSnapshot {id:format!("trae-{mode}-{id}"),client:"trae".into(),title,state:state.into(),label:label.into(),message:label.into(),updated_at:updated,created_at:created})
    }).collect()
}

fn read() -> Vec<SessionSnapshot> {
    let Ok(appdata) = std::env::var("APPDATA") else { return Vec::new() };
    let path = std::path::Path::new(&appdata).join("TRAE SOLO CN/User/globalStorage/state.vscdb");
    let Ok(db) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return Vec::new() };
    let _ = db.busy_timeout(Duration::from_millis(100));
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let Ok(mut query) = db.prepare("SELECT value FROM ItemTable WHERE key LIKE 'solo-lite-mode-state-map-%'") else { return Vec::new() };
    let rows = query.query_map([], |r| {
        // The column is stored as TEXT or BLOB; read the raw bytes either way.
        Ok(match r.get_ref(0)? {
            rusqlite::types::ValueRef::Text(text) => text.to_vec(),
            rusqlite::types::ValueRef::Blob(blob) => blob.to_vec(),
            _ => Vec::new(),
        })
    });
    let mut sessions = Vec::new();
    for bytes in rows.into_iter().flatten().flatten() {
        sessions.extend(parse_modes(&String::from_utf8_lossy(&bytes), now));
    }
    sessions
}
pub fn start() {
    std::thread::spawn(|| loop {
        if crate::presence::get_active_clients().iter().any(|c| c == "trae") {
            let rows = read();
            if let Ok(mut cache) = CACHE.lock() { *cache = Some((Instant::now(), rows)); }
        }
        std::thread::sleep(Duration::from_secs(2));
    });
    std::thread::spawn(|| loop {
        let expression=crate::sessions::resource("trae-status.js", include_str!("../resources/trae-status.js")).replace('\'',"''");
        let script=crate::sessions::resource("trae-status.ps1", include_str!("../resources/trae-status.ps1")).replace("'__TRAE_EXPRESSION__'",&format!("'{expression}'")).replace("param([int]$ParentPid)",&format!("$ParentPid = {}",std::process::id()));
        let mut cmd=Command::new("powershell.exe");
        cmd.args(["-NoProfile","-NonInteractive","-Command",&script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)] {use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
        if let Ok(mut child)=cmd.spawn(){
            if let Some(out)=child.stdout.take(){for line in BufReader::new(out).lines().map_while(Result::ok){
                let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                let rows=decode_live(&line,now);
                if let Ok(mut live)=LIVE.lock(){
                    // Missed probes (renderer busy) keep the previous state until the TTL ages it out.
                    if !rows.is_empty() || live.as_ref().map_or(true,|(at,_)|at.elapsed()>=Duration::from_secs(30)){*live=Some((Instant::now(),rows));}
                }
            }}
            let _=child.wait();
        }
        std::thread::sleep(Duration::from_secs(3));
    });
}
pub fn sessions() -> Vec<SessionSnapshot> {
    let mut rows = CACHE.lock().ok().and_then(|c| c.as_ref().filter(|(at,_)|at.elapsed().as_secs()<5).map(|(_,rows)|rows.clone())).unwrap_or_default();
    let live = LIVE.lock().ok().and_then(|c| c.as_ref().filter(|(at,_)|at.elapsed().as_secs()<5).map(|(_,rows)|rows.clone())).unwrap_or_default();
    rows.extend(live);
    rows
}
#[cfg(test)] mod tests {
    use super::*;
    // updateAt below maps to 2000s, so NOW=3000 keeps ages inside both freshness windows.
    const NOW: u64 = 3000;
    fn session(status: i64) -> String {
        format!(r#"{{"session":{{"sessionId":"a1","name":"修复登录","status":{status},"createdAt":1500000,"updateAt":2000000,"workspacePath":"D:\\3Dauto\\demo","source":{{"local_folder":"D:\\3Dauto\\demo"}}}}}}"#)
    }
    #[test] fn each_mode_with_a_session_yields_one_snapshot() {
        let rows = parse_modes(&format!(r#"{{"work":{},"code":{}}}"#, session(1), session(3)), NOW);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.id == "trae-work-a1"));
        assert!(rows.iter().any(|r| r.id == "trae-code-a1"));
        assert!(rows.iter().all(|r| r.client == "trae"));
        // Mode values without a session object are skipped defensively.
        let text = format!(r#"{{"work":"weird","code":{}}}"#, session(1));
        assert_eq!(parse_modes(&text, NOW).len(), 1);
        assert!(parse_modes("not json", NOW).is_empty());
    }
    #[test] fn numeric_status_maps_to_lamp_states() {
        let text = format!(r#"{{"s3":{},"s5":{},"s8":{},"s6":{},"s4":{},"s7":{}}}"#, session(3), session(5), session(8), session(6), session(4), session(7));
        let rows = parse_modes(&text, NOW);
        let of = |id: &str| rows.iter().find(|r| r.id == format!("trae-{id}-a1")).unwrap();
        assert_eq!(of("s3").state, "working");
        assert_eq!(of("s5").state, "completed");
        assert_eq!(of("s8").state, "waiting");
        assert_eq!(of("s6").state, "error");
        assert_eq!(of("s4").state, "completed");
        assert_eq!(of("s7").state, "idle");
        assert_eq!(of("s3").label, "运行中");
        assert_eq!(of("s8").label, "等待处理");
        assert_eq!(of("s7").label, "暂无状态 / 待确认");
        assert_eq!(of("s3").updated_at, 2000);
        assert_eq!(of("s3").created_at, 1500);
    }
    #[test] fn title_prefers_project_directory_name() {
        let rows = parse_modes(&format!(r#"{{"work":{}}}"#, session(1)), NOW);
        assert_eq!(rows[0].title, "demo · 修复登录");
        // Falls back to source.local_folder when workspacePath is absent.
        let local = r#"{"session":{"sessionId":"a2","name":"修复登录","status":1,"createdAt":1500000,"updateAt":2000000,"source":{"local_folder":"E:\\other\\proj"}}}"#;
        let rows = parse_modes(&format!(r#"{{"work":{local}}}"#), NOW);
        assert_eq!(rows[0].title, "proj · 修复登录");
        // No directory at all: the bare session name is used.
        let plain = r#"{"session":{"sessionId":"a4","name":"修复登录","status":1,"createdAt":1500000,"updateAt":2000000}}"#;
        assert_eq!(parse_modes(&format!(r#"{{"work":{plain}}}"#), NOW)[0].title, "修复登录");
    }
    #[test] fn missing_name_falls_back_to_generic_title() {
        let text = r#"{"work":{"session":{"sessionId":"a3","status":1,"updateAt":2000000,"workspacePath":"D:\\3Dauto\\demo"}}}"#;
        let rows = parse_modes(text, NOW);
        assert_eq!(rows[0].title, "TRAE 会话");
        assert_eq!(rows[0].created_at, rows[0].updated_at);
    }
    #[test] fn stale_snapshots_are_dropped_by_state() {
        let wrapped = |status: i64, now: u64| parse_modes(&format!(r#"{{"work":{}}}"#, session(status)), now);
        // working/error expire after 30 minutes.
        assert!(wrapped(1, 3801).is_empty());
        assert!(wrapped(6, 3801).is_empty());
        // completed/waiting/idle tolerate a full day.
        assert!(!wrapped(5, 3801).is_empty());
        assert!(wrapped(5, 2000 + 86_401).is_empty());
        assert!(wrapped(8, 2000 + 86_401).is_empty());
        assert!(wrapped(7, 2000 + 86_401).is_empty());
    }
    #[test] fn live_stream_counter_maps_to_working() {
        let row = decode_live(r#"[{"connected":true,"streams":2}]"#, 10).remove(0);
        assert_eq!(row.state, "working");
        assert_eq!(row.id, "trae-live");
        assert_eq!(row.message, "运行中 · 当前会话");
        assert_eq!(decode_live(r#"[{"connected":true,"streams":0}]"#, 10)[0].state, "idle");
        assert!(decode_live(r#"[{"connected":false,"streams":3}]"#, 10).is_empty());
        assert!(decode_live("[]", 10).is_empty());
        assert!(decode_live("not json", 10).is_empty());
    }
}
