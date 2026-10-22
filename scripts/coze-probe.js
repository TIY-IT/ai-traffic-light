(() => {
 const root=document.getElementById('root');
 const container=root[Object.keys(root).find(k=>k.startsWith('__reactContainer$'))];
 const pending=[container?.stateNode?.current||container],seen=new Set(),found=[];
 while(pending.length&&seen.size<30000){const f=pending.pop();if(!f||seen.has(f))continue;seen.add(f);pending.push(f.child,f.sibling);
 const p=f.memoizedProps;if(!p||typeof p!=='object')continue;
 if(p.clawChatAreaStore?.getState){const s=p.clawChatAreaStore.getState();found.push({store:'clawChatAreaStore',keys:Object.keys(s),scalars:Object.fromEntries(Object.entries(s).filter(([k,v])=>/status|running|streaming|generating/i.test(k)&&['string','number','boolean'].includes(typeof v))),objects:Object.fromEntries(Object.entries(s).filter(([k,v])=>v&&typeof v==='object').map(([k,v])=>[k,Object.keys(v).slice(0,40)]))});}
 if('sessionStatus' in p)found.push({sessionStatus:p.sessionStatus,sessionId:p.sessionId,conversationId:p.conversationId});
 if('isRunning' in p)found.push({isRunning:p.isRunning,keys:Object.keys(p)});
 }
 return {nodes:seen.size,found};
})()
