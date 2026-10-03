import { readFile, readdir, mkdir, mkdtemp, cp, rm, writeFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { hostTarget, sha } from './release-assets.mjs';
function run(cmd,args,options={}) { const r=spawnSync(cmd,args,{encoding:'utf8',...options}); if(r.error||r.status!==0)throw new Error(`${cmd} failed: ${r.stderr||r.error?.message}`); return r.stdout; }
const target=hostTarget(), bundle=resolve(`target/${target}/release/bundle`), output=resolve('unsigned-release');
await mkdir(output,{recursive:false});
try {
 const kinds=process.platform==='darwin'?['dmg']:process.platform==='win32'?['nsis']:['appimage','deb'];
 for(const kind of kinds) {
  const files=(await readdir(join(bundle,kind))).filter(n=>/\.(dmg|exe|AppImage|deb)$/.test(n));
  if(files.length!==1) throw new Error(`Expected exactly one ${kind} installer`);
  const artifact=join(bundle,kind,files[0]), temp=await mkdtemp(join(tmpdir(),'creativeshelf-installer-'));
  let mounted=false;
  try {
   let app=temp;
   if(kind==='dmg') {
    app=join(temp,'mount'); await mkdir(app);
    run('hdiutil',['attach','-readonly','-nobrowse','-mountpoint',app,artifact]); mounted=true;
    const apps=(await readdir(app)).filter(n=>n.endsWith('.app')); if(apps.length!==1)throw new Error('DMG app missing or duplicated'); app=join(app,apps[0]);
   } else if(kind==='nsis') run('7z',['x','-y',`-o${temp}`,artifact]);
   else if(kind==='deb') run('dpkg-deb',['-x',artifact,temp]);
   else { run(artifact,['--appimage-extract'],{cwd:temp}); app=join(temp,'squashfs-root'); }
   run(process.execPath,['scripts/unsigned-release.mjs','inspect',app,artifact],{stdio:'inherit'});
   await cp(artifact,join(output,files[0]));
   await cp(`release-reports/unsigned/${files[0]}.json`,join(output,`${files[0]}.inspection.json`));
  } finally {
   if(mounted)run('hdiutil',['detach',join(temp,'mount')]);
   await rm(temp,{recursive:true,force:true});
  }
 }
 await cp('release-reports/stage.json',join(output,`provenance-${target}.json`));
 const rows=[];
 for(const name of (await readdir(output)).sort()) rows.push(`${sha(await readFile(join(output,name)))}  ${name}`);
 await writeFile(join(output,`SHA256SUMS-${target}.txt`),rows.join('\n')+'\n');
} catch(e) { console.error(e.message); process.exitCode=1; }
