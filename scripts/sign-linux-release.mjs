import { readdir } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { signingSettings, signLinuxArtifact, verifyArtifact } from './release-signing.mjs';
// Detached GPG signatures for the inspected Linux downloads and their checksum list.
// The key must already be imported; its fingerprint comes from LINUX_SIGNING_FINGERPRINT.
export function signableNames(names) {
 const files=names.filter(n=>/\.(AppImage|deb)$/.test(n)||/^SHA256SUMS-.+\.txt$/.test(n)).sort();
 if(files.filter(n=>n.endsWith('.AppImage')).length!==1||files.filter(n=>n.endsWith('.deb')).length!==1) throw new Error('Expected exactly one AppImage and one deb installer');
 if(!files.some(n=>n.startsWith('SHA256SUMS-'))) throw new Error('Linux checksum list missing');
 if(names.some(n=>n.endsWith('.asc'))) throw new Error('Linux downloads are already signed');
 return files;
}
if(import.meta.url===pathToFileURL(process.argv[1]).href) {
 try {
  if(process.platform!=='linux') throw new Error('Linux release signing runs on Linux only');
  const settings=signingSettings(), directory=resolve(process.argv[2]||'unsigned-release');
  for(const name of signableNames(await readdir(directory))) {
   const path=join(directory,name);
   signLinuxArtifact(path,settings);
   verifyArtifact(path,undefined,settings);
  }
 } catch(e) { console.error(e.message); process.exitCode=1; }
}
