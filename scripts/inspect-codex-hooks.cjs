// Read-only diagnostics, limited to hook-engine logs (not conversation content).
const {DatabaseSync}=require('node:sqlite');
const path=require('node:path');
const db=new DatabaseSync(path.join(process.env.USERPROFILE,'.codex','logs_2.sqlite'),{readOnly:true});
try {
 console.log(db.prepare("SELECT ts,level,target,substr(feedback_log_body,1,900) body FROM logs WHERE target LIKE '%hook%' ORDER BY id DESC LIMIT 12").all());
} finally {db.close();}
