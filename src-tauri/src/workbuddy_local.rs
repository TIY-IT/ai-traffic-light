//! Optional compatibility with this machine's already-installed collector.
//! Never bundles or downloads that collector; opt-in flag required.
use crate::sessions::SessionSnapshot;
use std::{process::{Command,Stdio},time::{Duration,SystemTime,UNIX_EPOCH}};
fn directory()->Option<std::path::PathBuf>{std::env::var_os("LOCALAPPDATA").map(|p|std::path::PathBuf::from(p).join("AIStatusLight"))}
fn enabled()->bool{std::env::args().any(|a|a=="--legacy-local-workbuddy")}
pub fn start(){if !enabled(){return;}std::thread::spawn(||{
 let Some(dir)=directory() else{return;};let script=dir.join("status-monitor.ps1");if !script.is_file(){return;}
 let mut cmd=Command::new("powershell.exe");cmd.args(["-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-File"]).arg(script).arg("-BridgeParentPid").arg(std::process::id().to_string()).arg("-SnapshotPath").arg(dir.join("snapshot.json")).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
 #[cfg(windows)]{use std::os::windows::process::CommandExt;cmd.creation_flags(0x08000000);}
 if let Ok(mut child)=cmd.spawn(){let _=child.wait();}
});}
pub fn sessions()->Vec<SessionSnapshot>{
 if !enabled(){return vec![];}
 let Some(path)=directory().map(|d|d.join("snapshot.json")) else{return vec![];};
 let fresh=std::fs::metadata(&path).and_then(|m|m.modified()).ok().and_then(|m|m.elapsed().ok()).is_some_and(|age|age<Duration::from_secs(5));
 if !fresh{return vec![];}
 let Some(v)=std::fs::read_to_string(path).ok().and_then(|s|serde_json::from_str::<serde_json::Value>(s.trim_start_matches('\u{feff}')).ok()) else{return vec![];};
 let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
 v["tasks"].as_array().map(|tasks|tasks.iter().filter_map(|t|{
  let id=t["threadId"].as_str()?;if !id.starts_with("workbuddy:"){return None;}
  let(state,label)=match t["mode"].as_str()?{"working"|"executing"=>("working","运行中"),"waiting"=>("waiting","已停 / 待处理"),_=>("idle","暂无状态")};
  Some(SessionSnapshot{id:id.into(),client:"workbuddy".into(),title:t["title"].as_str().unwrap_or("WorkBuddy").chars().take(80).collect(),state:state.into(),label:label.into(),message:label.into(),created_at:0,updated_at:now})
 }).collect()).unwrap_or_default()
}
