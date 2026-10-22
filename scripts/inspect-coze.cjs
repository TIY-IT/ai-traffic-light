// Read-only, local desktop debugging. Never returns chat content or credentials.
const fs = require('node:fs');
async function evaluate(expression) {
  const targets = await (await fetch('http://127.0.0.1:19222/json/list', {signal:AbortSignal.timeout(3000)})).json();
  const target = targets.find(t => t.type === 'page' && t.url.startsWith('coze-desktop://-/'));
  if (!target || !target.webSocketDebuggerUrl.startsWith('ws://127.0.0.1:19222/')) throw Error('No local Coze target');
  return new Promise((resolve,reject) => {
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    const timer = setTimeout(()=>{socket.close();reject(Error('CDP timeout'));},5000);
    socket.onopen=()=>socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression,returnByValue:true}}));
    socket.onerror=()=>{clearTimeout(timer);reject(Error('CDP connection failed'));};
    socket.onmessage=event=>{const result=JSON.parse(event.data);if(result.id!==1)return;clearTimeout(timer);socket.close();if(result.error||result.result?.exceptionDetails)reject(Error('Evaluation failed'));else resolve(result.result.result.value);};
  });
}
module.exports={evaluate};
if(require.main===module) evaluate(fs.readFileSync(process.argv[2],'utf8')).then(x=>console.log(JSON.stringify(x,null,2))).catch(e=>{console.error(e.message);process.exitCode=1;});
