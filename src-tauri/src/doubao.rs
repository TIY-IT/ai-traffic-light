//! Opt-in desktop CDP adapter. Only reply counters are read; no message bodies.
use crate::sessions::SessionSnapshot;
use std::{io::{BufRead, BufReader}, process::{Command, Stdio}, sync::Mutex, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
static STATUS: Mutex<Option<(Instant, Vec<SessionSnapshot>)>> = Mutex::new(None);
fn decode(text: &str, now: u64) -> Vec<SessionSnapshot> {
    decode_with_memory(text, now, &mut false)
}

// `saw_activity` remembers that a reply was seen generating on this page, so builds without
// reply counters (DOM fallback) still show a stable red after the reply ends.
fn decode_with_memory(text: &str, now: u64, saw_activity: &mut bool) -> Vec<SessionSnapshot> {
    let Ok(rows) = serde_json::from_str::<serde_json::Value>(text) else { return vec![] };
    // The collector prints a one-element array; a bare object is tolerated too.
    let row = if rows.is_array() { match rows.get(0) { Some(row) => row, None => return vec![] } } else { &rows };
    if !row["connected"].as_bool().unwrap_or(false) { return vec![]; }
    let active = row["active"].as_i64().unwrap_or(0);
    let ended = row["ended"].as_i64().unwrap_or(0);
    if active > 0 { *saw_activity = true; }
    let (state,label) = if active > 0 {("working","运行中 · 当前会话")}
        else if ended > 0 || *saw_activity {("completed","已结束 · 当前会话")}
        else {("idle","暂无会话 · 已连接")};
    vec![SessionSnapshot {id:"doubao-current".into(),client:"doubao".into(),title:"豆包当前会话".into(),state:state.into(),label:label.into(),message:label.into(),created_at:now,updated_at:now}]
}
pub fn start() {
    std::thread::spawn(|| loop {
        let expression=crate::sessions::resource("doubao-status.js", include_str!("../resources/doubao-status.js")).replace('\'',"''");
        let script=crate::sessions::resource("doubao-status.ps1", include_str!("../resources/doubao-status.ps1")).replace("'__DOUBAO_EXPRESSION__'",&format!("'{expression}'")).replace("param([int]$ParentPid)",&format!("$ParentPid = {}",std::process::id()));
        let mut cmd=Command::new("powershell.exe");
        cmd.args(["-NoProfile","-NonInteractive","-Command",&script]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)] {use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
        if let Ok(mut child)=cmd.spawn(){
            let mut saw_activity=false;
            if let Some(out)=child.stdout.take(){for line in BufReader::new(out).lines().map_while(Result::ok){
                let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                let rows=decode_with_memory(&line,now,&mut saw_activity);
                if let Ok(mut status)=STATUS.lock(){
                    // Missed probes (renderer busy, page navigating) keep the previous state until the TTL ages it out.
                    if !rows.is_empty() || status.as_ref().map_or(true,|(at,_)|at.elapsed()>=Duration::from_secs(30)){*status=Some((Instant::now(),rows));}
                }
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
 #[test] fn reply_counters_map_to_lamp_states(){
  let line=|body:&str|format!("[{body}]");
  let row=decode(&line(r#"{"connected":true,"active":2,"ended":1}"#),10).remove(0);
  assert_eq!(row.state,"working");assert_eq!(row.id,"doubao-current");assert_eq!(row.client,"doubao");assert_eq!(row.title,"豆包当前会话");assert_eq!(row.message,"运行中 · 当前会话");
  assert_eq!(decode(&line(r#"{"connected":true,"active":0,"ended":3}"#),10)[0].state,"completed");
  assert_eq!(decode(&line(r#"{"connected":true,"active":0,"ended":0}"#),10)[0].state,"idle");
  assert!(decode(&line(r#"{"connected":false}"#),10).is_empty());
  assert!(decode("[]",10).is_empty());
  assert!(decode("not json",10).is_empty());
 }
 #[test] fn bare_object_snapshot_is_tolerated(){assert_eq!(decode(r#"{"connected":true,"ended":1}"#,10)[0].state,"completed");}
 #[test] fn dom_fallback_stays_red_after_a_generation(){
  let line=|body:&str|format!("[{body}]");
  let mut seen=false;
  assert_eq!(decode_with_memory(&line(r#"{"connected":true,"active":0,"ended":0}"#),10,&mut seen)[0].state,"idle");
  assert_eq!(decode_with_memory(&line(r#"{"connected":true,"active":1,"ended":0}"#),10,&mut seen)[0].state,"working");
  assert!(seen);
  // The DOM fallback has no ended counter; remembered activity keeps the lamp red after the reply.
  assert_eq!(decode_with_memory(&line(r#"{"connected":true,"active":0,"ended":0}"#),10,&mut seen)[0].state,"completed");
 }
}
