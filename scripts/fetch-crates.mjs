// Network recovery: fetch locked archives from the official CDN and verify SHA-256.
import fs from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
const cache = process.argv[2];
if (!cache) throw new Error('Pass the Cargo registry cache directory');
const lock = await fs.readFile('Cargo.lock', 'utf8');
const packages = lock.split('[[package]]').flatMap(block => {
  const name = block.match(/^name = "([^"]+)"/m)?.[1];
  const version = block.match(/^version = "([^"]+)"/m)?.[1];
  const checksum = block.match(/^checksum = "([^"]+)"/m)?.[1];
  return name && version && checksum ? [{name,version,checksum}] : [];
});
let completed = 0;
await Promise.all(Array.from({length:8}, async () => {
  while (packages.length) {
    const {name,version,checksum} = packages.pop();
    const file = path.join(cache, `${name}-${version}.crate`);
    try { await fs.access(file); continue; } catch {}
    const response = await fetch(`https://static.crates.io/crates/${name}/${name}-${version}.crate`);
    if (!response.ok) throw new Error(`${name}: ${response.status}`);
    const data = Buffer.from(await response.arrayBuffer());
    if (crypto.createHash('sha256').update(data).digest('hex') !== checksum) throw new Error(`Checksum mismatch: ${name}`);
    await fs.writeFile(file, data);
    completed++;
  }
}));
console.log(`Fetched and verified ${completed} locked archives`);
