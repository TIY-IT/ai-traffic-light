const assert=require('node:assert/strict');
(async()=>{
 const targets=await(await fetch('http://127.0.0.1:19223/json/list')).json();
 const target=targets.find(t=>t.url==='http://tauri.localhost/'&&t.title==='AI Traffic Light');assert.ok(target);
 const socket=new WebSocket(target.webSocketDebuggerUrl);
 const timer=setTimeout(()=>{socket.close();process.exitCode=1;},6000);
 socket.onopen=()=>socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{returnByValue:true,expression:`(()=>Object.fromEntries(['codex','coze','zcode'].map(client=>{const e=document.querySelector('[data-client="'+client+'"]');return [client,{visible:!e.hidden,red:e.querySelector('.light-red').classList.contains('active'),green:e.querySelector('.light-green').classList.contains('active'),status:e.querySelector('.client-status').textContent}]})))()`}}));
 socket.onmessage=e=>{const r=JSON.parse(e.data);if(r.id!==1)return;clearTimeout(timer);socket.close();const v=r.result?.result?.value;console.log(v);assert.equal(v.codex.visible,true);assert.equal(v.codex.green,true,'Current Codex task must be green without Hooks');assert.equal(v.codex.red,false);};
})().catch(e=>{console.error(e);process.exitCode=1;});
