const assert=require('node:assert/strict');
(async()=>{
 const targets=await(await fetch('http://127.0.0.1:19223/json/list')).json();
 const target=targets.find(t=>t.url==='http://tauri.localhost/'&&t.title==='AI Traffic Light');
 assert.ok(target);
 const socket=new WebSocket(target.webSocketDebuggerUrl);
 const timer=setTimeout(()=>{socket.close();process.exitCode=1;},5000);
 socket.onopen=()=>socket.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{returnByValue:true,expression:`(()=>{
 const widget=document.querySelector('.widget'),panel=document.querySelector('.panel-stack');
 const toggle=[...document.querySelectorAll('.details-toggle')].find(b=>!b.closest('.housing').hidden);
 const close=document.querySelector('#close-panel');
 if(widget.dataset.panelOpen==='true')close.click();
 toggle.click();const opened=getComputedStyle(panel).display==='flex';
 close.click();const closed=getComputedStyle(panel).display==='none';
 const dismissed=widget.dataset.panelDismissed==='true';
 toggle.click();const reopened=getComputedStyle(panel).display==='flex';
 document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}));
 return {opened,closed,dismissed,reopened,escapeClosed:getComputedStyle(panel).display==='none',persisted:localStorage.getItem('ai-light-panel-dismissed')==='true',lampVisible:[...document.querySelectorAll('.housing')].some(h=>!h.hidden),buttonSize:close.getBoundingClientRect().width};
 })()`}}));
 socket.onmessage=e=>{const r=JSON.parse(e.data);if(r.id!==1)return;clearTimeout(timer);socket.close();const v=r.result?.result?.value;console.log(v);for(const k of ['opened','closed','dismissed','reopened','escapeClosed','persisted','lampVisible'])assert.equal(v[k],true,k);};
})().catch(e=>{console.error(e);process.exitCode=1;});
