//! Distributable adapter: explicit hook signals, no legacy rollout scraper.
use crate::sessions::{SessionSnapshot, StatusSnapshot};
use std::{fs, path::Path, time::{SystemTime, UNIX_EPOCH}};
pub fn start() -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
fn hook_sessions(directory: &Path, now: u64) -> Vec<SessionSnapshot> {
    let Ok(entries) = fs::read_dir(directory) else { return Vec::new() };
    entries.flatten().filter_map(|entry| {
        let path = entry.path();
        if path.extension()?.to_str()? != "json" { return None; }
        let content = fs::read_to_string(&path).ok()?;
        let value: serde_json::Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).ok()?;
        let client = value["client"].as_str()?;
        if !["codex", "workbuddy"].contains(&client) { return None; }
        let stamp = value["timestamp"].as_u64()?;
        if now.saturating_sub(stamp) > 86400 { return None; }
        let (state,label) = match value["state"].as_str()? {
            "working" if now.saturating_sub(stamp) <= 1800 => ("working","运行中"),
            "completed" => ("completed","已结束"),
            "waiting" => ("waiting","等待处理"),
            "error" => ("error","异常"),
            _ => ("idle","暂无状态 / 待确认"),
        };
        let id = path.file_stem()?.to_str()?.to_owned();
        let cwd = value["cwd"].as_str().unwrap_or("");
        let project = Path::new(cwd).file_name().and_then(|p|p.to_str()).unwrap_or("");
        let title = if project.is_empty() {id.chars().take(24).collect()} else {format!("{} · {}",project,id.chars().rev().take(6).collect::<String>().chars().rev().collect::<String>())};
        Some(SessionSnapshot {id,client:client.into(),title,state:state.into(),label:label.into(),message:label.into(),updated_at:stamp,created_at:value["created_at"].as_u64().unwrap_or(stamp)})
    }).collect()
}
pub fn read() -> StatusSnapshot {
    let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let mut snapshot=StatusSnapshot::idle();
    if let Some(home)=dirs::home_dir(){snapshot.sessions=hook_sessions(&home.join(".ai-traffic-light/sessions"),now);}
    snapshot.sessions.extend(crate::marvis::sessions());
    snapshot.sessions.extend(crate::zcode::sessions());
    snapshot.sessions.extend(crate::coze::sessions());
    snapshot.sessions.extend(crate::trae::sessions());
    snapshot.sessions.extend(crate::doubao::sessions());
    let local=crate::codex_local::sessions();
    if !local.is_empty(){snapshot.sessions.retain(|s|s.client!="codex");snapshot.sessions.extend(local);}
    let buddy=crate::workbuddy_local::sessions();
    if !buddy.is_empty(){snapshot.sessions.retain(|s|s.client!="workbuddy");snapshot.sessions.extend(buddy);}
    snapshot.session_count=snapshot.sessions.len();snapshot.updated_at=now;
    if snapshot.sessions.iter().any(|s|s.state=="working") {snapshot.state="working".into();snapshot.label="运行中".into();}
    else if snapshot.sessions.iter().any(|s|["completed","waiting","error"].contains(&s.state.as_str())) {snapshot.state="completed".into();snapshot.label="已停止 · 查看结果".into();}
    else {snapshot.label="未接入 / 暂无状态".into();}
    snapshot
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn hooks_are_isolated_and_read_only(){
        let root=std::env::temp_dir().join(format!("light-hook-test-{}",std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let a=root.join("codex-a.json"); let b=root.join("workbuddy-b.json");
        fs::write(&a,r#"{"client":"codex","state":"working","timestamp":1000}"#).unwrap();
        fs::write(&b,r#"{"client":"workbuddy","state":"completed","timestamp":1000}"#).unwrap();
        let rows=hook_sessions(&root,1001);
        assert_eq!(rows.len(),2);assert!(rows.iter().any(|r|r.client=="codex"&&r.state=="working"));
        assert!(rows.iter().any(|r|r.client=="workbuddy"&&r.state=="completed"));
        assert!(hook_sessions(&root,100000).is_empty());assert!(a.exists()&&b.exists());
        fs::remove_file(a).unwrap();fs::remove_file(b).unwrap();fs::remove_dir(root).unwrap();
    }
}
