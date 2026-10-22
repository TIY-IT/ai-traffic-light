// Diagnostic only: reads the installed archive; does not copy vendor code into the app.
const fs=require('node:fs');
const fd=fs.openSync('C:/Users/Administrator/AppData/Local/Programs/Coze/versions/1.1.38/resources/app.asar','r');
try {
 const h=Buffer.alloc(16);fs.readSync(fd,h,0,16,0);const j=Buffer.alloc(h.readUInt32LE(12));fs.readSync(fd,j,0,j.length,16);const root=JSON.parse(j);let count=0;
 const term=process.argv[2];if(!term)throw Error('Search term required');
 function walk(o,p=''){for(const[k,v]of Object.entries(o.files||{})){const q=p+'/'+k;if(v.files)walk(v,q);else if(q.startsWith('/renderer/')&&q.endsWith('.js')){const b=Buffer.alloc(v.size);fs.readSync(fd,b,0,b.length,8+h.readUInt32LE(4)+Number(v.offset));const s=b.toString();let at=-1;while((at=s.indexOf(term,at+1))>=0){if(count++<15)console.log(q,s.slice(Math.max(0,at-250),at+400));}}}}walk(root);console.log('matches',count);
}finally{fs.closeSync(fd);}
