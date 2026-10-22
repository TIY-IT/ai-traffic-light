//! Opt-in desktop CDP adapter. Only mounted conversations are observable.
use crate::sessions::SessionSnapshot;
use std::{io::{BufRead, BufReader}, process::{Command, Stdio}, sync::Mutex, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
static STATUS: Mutex<Option<(Instant, Vec<SessionSnapshot>)>> = Mutex::new(None);
fn decode(text: &str, now: u64) -> Vec<SessionSnapshot> {
    let Ok(rows) = serde_json::from_str::<Vec<serde_json::Value>>(text) else { return vec![] };
    rows.iter().take(30).filter_map(|r| {
        let raw_id = r["id"].as_str().unwrap_or("");
        // Exact rows carry a numeric conversation id; the universal-hook fallback row is
        // flagged with live:true and reports a single aggregate state for the whole app.
        let id = if !raw_id.is_empty() && raw_id.len() <= 32 && raw_id.bytes().all(|b| b.is_ascii_digit()) {
            format!("coze-{raw_id}")
        } else if r["live"].as_bool().unwrap_or(false) {
            "coze-live".to_string()
        } else {
            return None;
        };
        let (state,label) = match r["status"].as_i64() { Some(1)=>("working","运行中 · 当前会话"),Some(2)=>("completed","已停止 · 当前会话"),_=>("idle","未知状态 · 当前会话") };
        Some(SessionSnapshot {id,client:"coze".into(),title:r["title"].as_str().unwrap_or("扣子当前会话").to_string(),state:state.into(),label:label.into(),message:label.into(),created_at:now,updated_at:now})
    }).collect()
}
pub fn start() {
    std::thread::spawn(|| loop {
        let expression=crate::sessions::resource("coze-status.js", include_str!("../resources/coze-status.js")).replace('\'',"''");
        let script=crate::sessions::resource("coze-status.ps1", include_str!("../resources/coze-status.ps1")).replace("'__COZE_EXPRESSION__'",&format!("'{expression}'")).replace("param([int]$ParentPid)",&format!("$ParentPid = {}",std::process::id()));
        let mut cmd=Command::new("powershell.exe");
        cmd.args(["-NoProfile","-NonInteractive","-Command",&script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)] {use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
        if let Ok(mut child)=cmd.spawn(){
            if let Some(out)=child.stdout.take(){for line in BufReader::new(out).lines().map_while(Result::ok){
                let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                if let Ok(mut status)=STATUS.lock(){*status=Some((Instant::now(),decode(&line,now)));}
            }}
            let _=child.wait();
        }
        std::thread::sleep(Duration::from_secs(3));
    });
}
pub fn sessions()->Vec<SessionSnapshot>{
    STATUS.lock().ok().and_then(|s|s.as_ref().filter(|(at,_)|at.elapsed()<Duration::from_secs(5)).map(|(_,rows)|rows.clone())).unwrap_or_default()
}
#[cfg(test)] mod tests {
 use super::*;
 #[test] fn explicit_status_only(){let rows=decode(r#"[{"id":"12","status":1},{"id":"13","status":2},{"id":"14","status":99}]"#,10);assert_eq!(rows.iter().map(|r|r.state.as_str()).collect::<Vec<_>>(),vec!["working","completed","idle"]);assert!(decode("not json",10).is_empty());assert!(decode(r#"[{"id":"../bad","status":1}]"#,10).is_empty());
 assert_eq!(decode(r#"[{"id":"coze-live","live":true,"status":1,"title":"x"}]"#,10)[0].id,"coze-live");
 assert_eq!(decode(r#"[{"id":"coze-live","live":true,"status":1,"title":"x"}]"#,10)[0].state,"working");
 assert_eq!(decode(r#"[{"id":"coze-live","live":true,"status":2}]"#,10)[0].state,"completed");
 assert!(decode(r#"[{"id":"coze-live","status":1}]"#,10).is_empty());}
}
