const {DatabaseSync}=require('node:sqlite');
const fs=require('node:fs'),path=require('node:path');
const root=path.join(process.env.APPDATA,'Tencent/Marvis/User');
for(const user of fs.readdirSync(root)) for(const name of ['data.db']) {
 const file=path.join(root,user,'database',name); if(!fs.existsSync(file))continue;
 try {const db=new DatabaseSync(file,{readOnly:true}); console.log(name, user.slice(-4),db.prepare("select status,count(*) as n from conversations group by status").all()); console.log(db.prepare("select event_type,count(*) n from agui_events group by event_type").all()); console.log(db.prepare("select conversation_id,status,updated_at from conversations order by updated_at desc limit 3").all()); console.log(db.prepare("select event_type,timestamp from agui_events order by rowid desc limit 8").all()); db.close();} catch(e){console.log(name,e.message);}
}
