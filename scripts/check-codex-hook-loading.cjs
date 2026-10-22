// Ask a standalone local server to parse hooks. Creates no task and changes no trust.
const {spawn}=require('node:child_process');
const readline=require('node:readline');
const exe=process.argv[2];if(!exe)throw Error('Codex executable path required');
const child=spawn(exe,['app-server','--stdio'],{windowsHide:true,stdio:['pipe','pipe','pipe']});
const send=value=>child.stdin.write(JSON.stringify(value)+'\n');
const timeout=setTimeout(()=>{child.kill();console.error('Hook inspection timed out');process.exitCode=1;},12000);
readline.createInterface({input:child.stdout}).on('line',line=>{
 let value;try{value=JSON.parse(line);}catch{return;}
 if(value.id===1){if(value.error){console.error(value.error);clearTimeout(timeout);child.kill();return;}send({method:'initialized'});send({id:2,method:'hooks/list',params:{cwds:['D:\\3Dauto\\desktop-pet']}});}
 if(value.id===2){const summary=value.result?.data?.map(entry=>({cwd:entry.cwd,warnings:entry.warnings,errors:entry.errors,hooks:entry.hooks.map(h=>({event:h.eventName,enabled:h.enabled,trustStatus:h.trustStatus,sourcePath:h.sourcePath}))}));console.log(JSON.stringify(summary||value.error,null,2));clearTimeout(timeout);child.stdin.end();child.kill();}
});
send({id:1,method:'initialize',params:{clientInfo:{name:'light-hook-diagnostic',version:'1.0'},capabilities:{experimentalApi:true}}});
