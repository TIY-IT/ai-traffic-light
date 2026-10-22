use crate::sessions::SessionSnapshot;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{sync::Mutex, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
static CACHE: Mutex<Option<(Instant, Vec<SessionSnapshot>)>> = Mutex::new(None);

fn classify(role: &str, finish: &str, error: bool, age: u64) -> (&'static str, &'static str) {
    if error { return ("error", "异常 / 已停止"); }
    if role == "assistant" && matches!(finish, "stop" | "completed" | "end_turn" | "length") { return ("completed", "已结束"); }
    if age > 120_000 { return ("idle", "状态过期 / 待确认"); }
    if role == "user" || (role == "assistant" && (finish.is_empty() || finish == "tool-calls")) { return ("working", "运行中"); }
    ("idle", "状态未知")
}

// Follow descendants, including grandchildren. UNION also bounds malformed cycles.
// Ignore child activity belonging to an earlier root user turn.
fn descendants(db: &Connection, root: &str, cutoff: u64, now: u64) -> rusqlite::Result<(usize, usize)> {
    let mut q=db.prepare("WITH RECURSIVE tree(id) AS (SELECT id FROM session WHERE parent_id=?1 UNION SELECT s.id FROM session s JOIN tree t ON s.parent_id=t.id) SELECT s.id,s.time_updated FROM tree t JOIN session s ON s.id=t.id WHERE s.id!=?1")?;
    let ids=q.query_map([root],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?.max(0) as u64)))?;
    let (mut working,mut unknown)=(0,0);
    for id in ids {
        let (id,session_stamp)=id?;
        let m=db.query_row("SELECT json_extract(data,'$.role'),coalesce(json_extract(data,'$.finish'),''),json_type(data,'$.error') IS NOT NULL AND json_type(data,'$.error')!='null',time_updated FROM message WHERE session_id=?1 ORDER BY sequence DESC LIMIT 1",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,r.get::<_,i64>(3)?.max(0) as u64))).optional()?;
        if let Some((role,finish,error,stamp))=m {
            let part:Option<i64>=db.query_row("SELECT max(time_updated) FROM part WHERE session_id=?1",[&id],|r|r.get(0))?;
            let stamp=stamp.max(part.unwrap_or(0).max(0) as u64);
            if stamp<cutoff {continue;}
            match classify(&role,&finish,error,now.saturating_sub(stamp)).0 {"working"=>working+=1,"idle"=>unknown+=1,_=>{}}
        } else if session_stamp>=cutoff {unknown+=1;}
    }
    Ok((working,unknown))
}

fn read(path: &std::path::Path) -> rusqlite::Result<Vec<SessionSnapshot>> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    db.busy_timeout(Duration::from_millis(100))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let mut query = db.prepare("SELECT id,title,time_updated FROM session WHERE parent_id IS NULL AND time_archived IS NULL ORDER BY time_updated DESC LIMIT 30")?;
    let rows = query.query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?.max(0) as u64)))?;
    let mut sessions = Vec::new();
    for row in rows {
        let (id,title,updated) = row?;
        if now.saturating_sub(updated) > 86_400_000 { continue; }
        // Project only status metadata in SQL; do not load message or tool contents.
        let message = db.query_row("SELECT json_extract(data,'$.role'),coalesce(json_extract(data,'$.finish'),''),json_type(data,'$.error') IS NOT NULL AND json_type(data,'$.error') != 'null',time_updated FROM message WHERE session_id=?1 ORDER BY sequence DESC LIMIT 1", [&id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,r.get::<_,i64>(3)?.max(0) as u64))).optional()?;
        if let Some((role,finish,error,stamp)) = message {
            let part_stamp: Option<i64> = db.query_row("SELECT max(time_updated) FROM part WHERE session_id=?1", [&id], |r| r.get(0))?;
            let stamp = stamp.max(part_stamp.unwrap_or(0).max(0) as u64);
            let (state,label) = classify(&role,&finish,error,now.saturating_sub(stamp));
            let cutoff:Option<i64>=db.query_row("SELECT max(time_updated) FROM message WHERE session_id=?1 AND json_extract(data,'$.role')='user'",[&id],|r|r.get(0))?;
            let (children,unknown)=descendants(&db,&id,cutoff.unwrap_or(updated as i64).max(0) as u64,now)?;
            let child_label=format!("运行中 · {} 个子代理",children);
            let (state,label)=if children>0 {("working",child_label.as_str())} else if unknown>0 && state=="completed" {("idle","子代理状态待确认")} else {(state,label)};
            sessions.push(SessionSnapshot {id:format!("zcode:{id}"),client:"zcode".into(),title:title.chars().take(80).collect(),state:state.into(),label:label.into(),message:label.into(),updated_at:stamp/1000,created_at:0});
        }
    }
    Ok(sessions)
}
pub fn start() {
    std::thread::spawn(|| loop {
        if crate::presence::get_active_clients().iter().any(|c|c=="zcode") {
            if let Some(home) = dirs::home_dir() {
                if let Ok(rows) = read(&home.join(".zcode/cli/db/db.sqlite")) {
                    if let Ok(mut cache)=CACHE.lock(){*cache=Some((Instant::now(),rows));}
                }
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    });
}
pub fn sessions() -> Vec<SessionSnapshot> {
    CACHE.lock().ok().and_then(|c| c.as_ref().filter(|(at,_)|at.elapsed().as_secs()<5).map(|(_,rows)|rows.clone())).unwrap_or_default()
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn tool_finish_is_not_turn_finish(){
        assert_eq!(classify("assistant","tool-calls",false,1).0,"working");
        assert_eq!(classify("assistant","",false,1).0,"working");
        assert_eq!(classify("assistant","stop",false,1).0,"completed");
        assert_eq!(classify("assistant","completed",false,1).0,"completed");
        assert_eq!(classify("assistant","",true,1).0,"error");
        assert_eq!(classify("assistant","",false,130000).0,"idle");
    }
    #[test] fn local_metadata_read(){
        let path=dirs::home_dir().unwrap().join(".zcode/cli/db/db.sqlite");
        if path.exists(){let rows=read(&path).unwrap();println!("ZCode metadata: {:?}",rows.iter().map(|r|&r.state).collect::<Vec<_>>());}
    }
    #[test] fn parent_end_does_not_hide_running_descendants(){
        let db=Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE session(id TEXT,parent_id TEXT,time_updated INTEGER);CREATE TABLE message(session_id TEXT,data TEXT,time_updated INTEGER,sequence INTEGER);CREATE TABLE part(session_id TEXT,time_updated INTEGER);INSERT INTO session VALUES ('root',NULL,1000),('child','root',1000),('grandchild','child',1100),('old','root',100);INSERT INTO message VALUES ('child','{\"role\":\"assistant\",\"finish\":\"stop\"}',1000,1),('grandchild','{\"role\":\"assistant\",\"finish\":\"tool-calls\"}',1100,1),('old','{\"role\":\"assistant\"}',100,1);").unwrap();
        assert_eq!(descendants(&db,"root",900,1200).unwrap(),(1,0));
        db.execute("UPDATE message SET data='{\"role\":\"assistant\",\"finish\":\"stop\"}' WHERE session_id='grandchild'",[]).unwrap();
        assert_eq!(descendants(&db,"root",900,1300).unwrap(),(0,0));
        db.execute("UPDATE message SET data='{\"role\":\"assistant\"}' WHERE session_id='grandchild'",[]).unwrap();
        assert_eq!(descendants(&db,"root",900,200000).unwrap(),(0,1));
    }
    #[test] fn database_to_parent_status_regression(){
        let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let path=std::env::temp_dir().join(format!("zcode-tree-{}-{now}.sqlite",std::process::id()));
        let db=Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE session(id TEXT,parent_id TEXT,title TEXT,time_updated INTEGER,time_archived INTEGER);CREATE TABLE message(session_id TEXT,data TEXT,time_updated INTEGER,sequence INTEGER);CREATE TABLE part(session_id TEXT,time_updated INTEGER);").unwrap();
        for(id,parent)in[("root",None),("child",Some("root")),("grandchild",Some("child"))]{db.execute("INSERT INTO session VALUES(?1,?2,?1,?3,NULL)",rusqlite::params![id,parent,now]).unwrap();}
        db.execute("INSERT INTO message VALUES('root','{\"role\":\"user\"}',?1,0)",[now-1000]).unwrap();
        for(id,finish)in[("root","stop"),("child","stop"),("grandchild","tool-calls")]{db.execute("INSERT INTO message VALUES(?1,?2,?3,1)",rusqlite::params![id,serde_json::json!({"role":"assistant","finish":finish}).to_string(),now]).unwrap();}
        let rows=read(&path).unwrap();assert_eq!(rows.len(),1);assert_eq!(rows[0].state,"working");assert!(rows[0].label.contains("1 个子代理"));
        db.execute("UPDATE message SET data='{\"role\":\"assistant\",\"finish\":\"stop\"}' WHERE session_id='grandchild'",[]).unwrap();
        assert_eq!(read(&path).unwrap()[0].state,"completed");
        db.execute("INSERT INTO session VALUES('new-child','root','new',?1,NULL)",[now]).unwrap();
        assert_eq!(read(&path).unwrap()[0].state,"idle","Child without first event must not turn red");
        drop(db);std::fs::remove_file(path).unwrap();
    }
}
