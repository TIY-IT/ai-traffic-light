const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const {spawn}=require('node:child_process');
const expression=fs.readFileSync('src-tauri/resources/coze-status.js','utf8');
function fixture(props){let head=null;for(const p of props)head={memoizedProps:p,sibling:head};const root={__reactContainer$test:{stateNode:{current:{child:head}}}};return vm.runInNewContext(expression,{document:{getElementById:()=>root},location:{protocol:'coze-desktop:',href:'coze-desktop://-/session/11'},URL});}
assert.equal(fixture([{sessionId:'11',conversationId:'22',sessionStatus:1}])[0].status,1);
assert.equal(fixture([{sessionId:'11',conversationId:'22',sessionStatus:2}])[0].status,2);
assert.equal(fixture([{sessionId:'11',sessionStatus:1},{sessionId:'11',sessionStatus:2}])[0].status,-1);
assert.equal(fixture([{sessionId:'bad',sessionStatus:1}]).length,0);
assert.equal(fixture([{isRunning:true}]).length,0);
assert.equal(fixture([]).length,0);
console.log('Coze metadata fixture tests passed');
if(process.argv.includes('--live')){
 const script=fs.readFileSync('src-tauri/resources/coze-status.ps1','utf8').replace('param([int]$ParentPid)',`$ParentPid = ${process.pid}`).replace("'__COZE_EXPRESSION__'",()=>"'"+expression.replaceAll("'","''")+"'");
 const child=spawn('powershell.exe',['-NoProfile','-NonInteractive','-Command',script],{windowsHide:true});
 const timer=setTimeout(()=>{child.kill();console.error('Live collector timeout');process.exitCode=1;},10000);
 let text='';child.stdout.on('data',chunk=>{text+=chunk;const pos=text.indexOf('\n');if(pos<0)return;clearTimeout(timer);child.kill();try{const rows=JSON.parse(text.slice(0,pos));assert.ok(rows.length>0,'No live conversation metadata');assert.ok(rows.every(r=>/^\d+$/.test(r.id)&&typeof r.status==='number'));console.log('Live PowerShell CDP collector passed:',rows.map(r=>({status:r.status,title:r.title})));}catch(e){console.error(e.message);process.exitCode=1;}});
 child.stderr.on('data',chunk=>process.stderr.write(chunk));
}
