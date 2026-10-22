//! Local read-only task lifecycle adapter. Does not require Hooks or parse answers as completion.
use crate::sessions::SessionSnapshot;
use rusqlite::{Connection,OpenFlags};
use std::{collections::{HashMap,HashSet},fs::File,io::{Read,Seek,SeekFrom},path::Path,sync::Mutex,time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
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
 if crate::presence::get_active_clients().iter().any(|c|c=="codex"){
  if let Some(home)=dirs::home_dir(){if let Ok(rows)=read(&home.join(".codex")){if let Ok(mut cache)=CACHE.lock(){*cache=Some((Instant::now(),rows));}}}
 }
 std::thread::sleep(Duration::from_millis(500));
});}
pub fn sessions()->Vec<SessionSnapshot>{CACHE.lock().ok().and_then(|c|c.as_ref().filter(|(at,_)|at.elapsed()<Duration::from_secs(5)).map(|(_,r)|r.clone())).unwrap_or_default()}
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
}
