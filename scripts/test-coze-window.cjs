// Read-only integration check against our own local WebView2 debug target.
const assert=require('node:assert/strict');
(async()=>{
 const targets=await(await fetch('http://127.0.0.1:19223/json/list')).json();
 const target=targets.find(t=>t.url==='http://tauri.localhost/'&&t.title==='AI Traffic Light');
 assert.ok(target,'Light window not found');
 const socket=new WebSocket(target.webSocketDebuggerUrl);
 const timer=setTimeout(()=>{socket.close();process.exitCode=1;},5000);
 socket.onopen=()=>socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{returnByValue:true,expression:`(()=>{const h=document.querySelector('[data-client="coze"]');return {hidden:h?.hidden,lights:[...h.querySelectorAll('.light')].map(l=>({classes:l.className})),status:h.querySelector('.client-status').textContent,rows:[...document.querySelectorAll('.session-list li')].map(e=>e.textContent),width:innerWidth}})()`}}));
 socket.onmessage=event=>{const r=JSON.parse(event.data);if(r.id!==1)return;clearTimeout(timer);socket.close();const value=r.result?.result?.value;console.log(JSON.stringify(value,null,2));assert.equal(value.hidden,false);assert.ok(value.lights[0].classes.split(' ').includes('active'),'Stopped Coze should be red');assert.ok(!value.lights[1].classes.split(' ').includes('active'),'Stopped Coze must not be green');};
})().catch(e=>{console.error(e.message);process.exitCode=1;});
