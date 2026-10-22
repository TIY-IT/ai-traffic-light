const fs=require('node:fs'),p=require('node:path'),cp=require('node:child_process');
const destination=process.argv[2];
const metadata=JSON.parse(cp.execFileSync('E:/Rust/cargo/bin/cargo.exe',['metadata','--offline','--format-version','1','--manifest-path','src-tauri/Cargo.toml'],{maxBuffer:20e6,encoding:'utf8'}));
const dir=p.join(destination,'LICENSES');fs.mkdirSync(dir,{recursive:true});
const notices=['Upstream UI and hook writer: https://github.com/pcj1997/ai-traffic-light (MIT; see LICENSE).','Rust dependency notices (conservative superset, including build-time dependencies). Corresponding source archives accompany MPL components and packages with nested license notices.'];
let missing=[];
for(const item of metadata.packages){
 if(!item.source)continue;
 const root=p.dirname(item.manifest_path);
 const files=fs.readdirSync(root).filter(n=>/^(license|copying|notice|unlicense)/i.test(n)&&fs.statSync(p.join(root,n)).isFile());
 notices.push(`${item.name} ${item.version}: ${item.license || 'SEE LICENSE FILE'} ${item.repository || ''}`);
 if(item.license_file && !files.includes(item.license_file) && fs.existsSync(p.join(root,item.license_file)))files.push(item.license_file);
 for(const name of files)fs.copyFileSync(p.join(root,name),p.join(dir,`${item.name}-${item.version}-${p.basename(name)}`));
 if(!files.length || /MPL|BSD|ISC/.test(item.license||'')){
   const cache=p.resolve(root,'../../../cache',p.basename(p.dirname(root)),`${item.name}-${item.version}.crate`);
   if(!fs.existsSync(cache))throw new Error(`Source archive missing: ${cache}`);
   const sourceDir=p.join(destination,'SOURCES');fs.mkdirSync(sourceDir,{recursive:true});
   fs.copyFileSync(cache,p.join(sourceDir,p.basename(cache)));
   missing.push(`${item.name}: source archive included`);
 }
}
const api='node_modules/@tauri-apps/api';for(const name of fs.readdirSync(api).filter(n=>/^license/i.test(n)))fs.copyFileSync(p.join(api,name),p.join(dir,'tauri-js-'+name));
fs.writeFileSync(p.join(destination,'THIRD-PARTY.txt'),notices.join('\r\n'));
console.log('Additional source archives included:',missing.length);
