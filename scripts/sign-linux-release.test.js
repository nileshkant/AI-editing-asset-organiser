import { mkdtemp, mkdir, writeFile, readdir, rm, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { it, expect } from 'vitest';
import { signableNames } from './sign-linux-release.mjs';
const target='SHA256SUMS-x86_64-unknown-linux-gnu.txt';
it('signs both Linux installers and their checksum list, nothing else',()=>{
 expect(signableNames(['CreativeShelf_1.0.0_amd64.AppImage','CreativeShelf_1.0.0_amd64.deb',target,'provenance-x86_64-unknown-linux-gnu.json','CreativeShelf_1.0.0_amd64.deb.inspection.json']))
  .toEqual(['CreativeShelf_1.0.0_amd64.AppImage','CreativeShelf_1.0.0_amd64.deb',target]);
});
it('refuses incomplete, duplicated or already-signed Linux downloads',()=>{
 expect(()=>signableNames(['a.AppImage',target])).toThrow('exactly one');
 expect(()=>signableNames(['a.AppImage','b.AppImage','a.deb',target])).toThrow('exactly one');
 expect(()=>signableNames(['a.AppImage','a.deb'])).toThrow('checksum list missing');
 expect(()=>signableNames(['a.AppImage','a.deb',target,'a.deb.asc'])).toThrow('already signed');
});
const gpgAvailable=process.platform==='linux'&&spawnSync('gpg',['--version']).status===0;
// Exercises the key recipe documented in docs/RELEASE.md with a throwaway key.
it.skipIf(!gpgAvailable)('signs with a documented signing-only key and verifies its published fingerprint',async()=>{
 const root=await mkdtemp(join(tmpdir(),'creativeshelf-gpg-test-')), home=join(root,'gnupg'), out=join(root,'unsigned-release');
 try {
  await mkdir(home); await chmod(home,0o700); await mkdir(out);
  const env={...process.env,GNUPGHOME:home};
  const gpg=args=>spawnSync('gpg',['--batch',...args],{env,encoding:'utf8'});
  expect(gpg(['--pinentry-mode','loopback','--passphrase','','--quick-gen-key','CreativeShelf Test <test@example.invalid>','ed25519','sign','1d']).status).toBe(0);
  const exported=gpg(['--armor','--export','CreativeShelf Test']).stdout, key=join(root,'key.asc');
  await writeFile(key,exported);
  const fingerprint=gpg(['--show-keys','--with-colons',key]).stdout.split('\n').find(l=>l.startsWith('fpr:')).split(':')[9];
  for(const name of ['a.AppImage','a.deb',target,'provenance.json']) await writeFile(join(out,name),name);
  const sign=spawnSync(process.execPath,[resolve('scripts/sign-linux-release.mjs'),out],{env:{...env,LINUX_SIGNING_FINGERPRINT:fingerprint},encoding:'utf8'});
  expect(sign.stderr).toBe(''); expect(sign.status).toBe(0);
  expect((await readdir(out)).filter(n=>n.endsWith('.asc')).sort()).toEqual(['a.AppImage.asc','a.deb.asc',target+'.asc']);
  await writeFile(join(out,'a.deb'),'tampered');
  expect(gpg(['--verify',join(out,'a.deb.asc'),join(out,'a.deb')]).status).not.toBe(0);
 } finally { await rm(root,{recursive:true,force:true}); }
},60000);
