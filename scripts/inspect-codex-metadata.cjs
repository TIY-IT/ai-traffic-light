const {DatabaseSync}=require('node:sqlite');const fs=require('node:fs');const path=require('node:path');
const d=new DatabaseSync(path.join(process.env.USERPROFILE,'.codex/state_5.sqlite'),{readOnly:true});
try {
 const rows=d.prepare("SELECT id,source,agent_path,thread_source,rollout_path FROM threads WHERE agent_path IS NOT NULL OR source LIKE '%subagent%' ORDER BY updated_at DESC LIMIT 4").all();
 for(const row of rows){
  console.log({id:row.id,source:row.source,agent_path:row.agent_path,thread_source:row.thread_source});
  if(!row.rollout_path||!fs.existsSync(row.rollout_path))continue;
  const fd=fs.openSync(row.rollout_path,'r');const size=fs.fstatSync(fd).size;const b=Buffer.alloc(Math.min(size,262144));fs.readSync(fd,b,0,b.length,size-b.length);fs.closeSync(fd);
  const events=[];for(const line of b.toString().split('\n')){try{const x=JSON.parse(line);if(x.type==='event_msg')events.push({type:x.payload?.type,turn_id:x.payload?.turn_id});}catch{}}
  console.log(events.filter(e=>/task|turn|agent|abort|complete/.test(e.type)).slice(-15));
 }
}finally{d.close();}
