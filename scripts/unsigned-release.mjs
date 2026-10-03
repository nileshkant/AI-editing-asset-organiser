import { readFile, writeFile, mkdir, readdir, mkdtemp, cp, rm, rename } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { tmpdir, homedir } from 'node:os';
import { spawnSync } from 'node:child_process';
import { rustEnv } from './cargo.mjs';
import { hostTarget, readLock, approvedMedia, qualifyTools, stageResources, inventory, inspectPackage, sha, binaryArchitecture } from './release-assets.mjs';
const target = hostTarget();
function run(cmd,args,options={}) {
  const { env: extraEnv, ...spawnOptions } = options;
  const env = { ...rustEnv, ...extraEnv };
  for (const key of Object.keys(env)) if (/^(APPLE_|WINDOWS_CERT_|TAURI_SIGNING_|LDAI_SIGN|SIGN(?:_|$))/.test(key)) delete env[key];
  // AppImage tooling otherwise strips staged helper executables after their hashes are recorded.
  // Keep exact audited bytes; package inspection still rejects any unexpected change.
  if (process.platform === 'linux') env.NO_STRIP = '1';
  const r = spawnSync(cmd,args,{env,encoding:'utf8',...spawnOptions});
  if (r.error || r.status !== 0) throw new Error(`${cmd} failed: ${r.stderr || r.error?.message}`);
  return r.stdout;
}
async function normalizedMac(path) {
  const temp = await mkdtemp(join(tmpdir(),'creativeshelf-adhoc-'));
  try { const file=join(temp,'binary'); await cp(path,file); run('codesign',['--remove-signature',file]); return sha(await readFile(file)); }
  finally { await rm(temp,{recursive:true,force:true}); }
}
try {
 const action=process.argv[2];
 if (action==='build') {
  const dirty=run('git',['status','--porcelain']);
  if(dirty.trim()) throw new Error('Unsigned tester builds require a clean committed checkout');
  const lock=await readLock('.release-input/media-lock.json');
  const source=JSON.parse(await readFile('release/ffmpeg-source.json','utf8'));
  if(lock.targets[target].sourceSha256!==source.sha256) throw new Error('Media source lock mismatch');
  const media=await approvedMedia(lock,target,resolve('.release-input')); qualifyTools(media,target);
  run(process.execPath,['scripts/cargo.mjs','build','-p','soundshelf-agent','--bin','soundshelf-mcp','--release','--locked','--target',target],{stdio:'inherit'});
  const suffix=process.platform==='win32'?'.exe':'';
  const stage=await stageResources(media,await readFile(`target/${target}/release/soundshelf-mcp${suffix}`),target,resolve('src-tauri/release-assets'));
  const bundle={resources:stage.resources,targets:process.platform==='darwin'?['app','dmg']:process.platform==='win32'?['nsis']:['appimage','deb']};
  bundle.icon=process.platform==='win32'?['icons/icon.ico']:['icons/icon.png'];
  if(process.platform==='darwin') {
   for(const file of [`media/ffmpeg`,`media/ffprobe`,`mcp/soundshelf-mcp`]) run('codesign',['--force','--sign','-',resolve('src-tauri/release-assets',file)]);
   bundle.macOS={signingIdentity:'-',minimumSystemVersion:'14.2'};
  }
  if(process.platform==='win32') bundle.windows={allowDowngrades:false,webviewInstallMode:{type:'offlineInstaller'}};
  stage.entries=await inventory('src-tauri/release-assets'); stage.bytes=stage.entries.reduce((n,e)=>n+e.bytes,0);
  await mkdir('release-reports',{recursive:true});
  await writeFile('release-reports/stage.json',JSON.stringify({...stage,resources:undefined,target,purpose:'unsigned-tester-prerelease',workspaceDirty:false,commit:run('git',['rev-parse','HEAD']).trim(),cargoLockSha256:sha(await readFile('Cargo.lock')),npmLockSha256:sha(await readFile('package-lock.json'))},null,2));
  const version=(process.env.GITHUB_REF_NAME?.startsWith('v') ? process.env.GITHUB_REF_NAME.slice(1) : '0.1.0-alpha.1');
  if(!/^\d+\.\d+\.\d+-alpha\.\d+$/.test(version)) throw new Error('Invalid preview version');
  await writeFile('src-tauri/tauri.staged.conf.json',JSON.stringify({version,bundle},null,2));
  run(process.execPath,['scripts/desktop.mjs','build','--target',target,'--config','src-tauri/tauri.staged.conf.json',...(process.platform==='darwin'?[]:['--no-sign'])],{stdio:'inherit'});
  if (process.platform === 'linux') {
   // linuxdeploy rewrites ELF RPATHs inside usr/lib, including our resource bridge.
   // Its dependency deployment is complete: restore audited resources before the final
   // squashfs emission, then independently extract and inspect that final artifact.
   const directory=resolve(`target/${target}/release/bundle/appimage`);
   const appdir=join(directory,'CreativeShelf.AppDir');
   const members=await inventory(appdir,true);
   for (const expected of stage.entries) {
    const matches=members.filter(e=>e.path===expected.path||e.path.endsWith('/'+expected.path));
    if(matches.length!==1||matches[0].kind==='symlink') throw new Error(`AppDir resource missing, linked or duplicated: ${expected.path}`);
    const original=resolve('src-tauri/release-assets',expected.path);
    if(sha(await readFile(original))!==expected.sha256) throw new Error('Staged resource changed before final AppImage emission');
    await cp(original,join(appdir,matches[0].path));
   }
   const installers=(await readdir(directory)).filter(n=>n.endsWith('.AppImage'));
   if(installers.length!==1) throw new Error('Expected one generated AppImage');
   const artifact=join(directory,installers[0]), rebuilt=artifact+'.rebuilt';
   const plugin=join(process.env.XDG_CACHE_HOME||join(homedir(),'.cache'),'tauri','linuxdeploy-plugin-appimage.AppImage');
   run(plugin,['--appimage-extract-and-run','--appdir',appdir],{env:{...rustEnv,APPIMAGE_EXTRACT_AND_RUN:'1',ARCH:'x86_64',OUTPUT:rebuilt},stdio:'inherit'});
   await rename(rebuilt,artifact);
  }

 } else if(action==='inspect') {
  const app=resolve(process.argv[3]),artifact=resolve(process.argv[4]);
  const stage=JSON.parse(await readFile('release-reports/stage.json','utf8'));
  if(stage.purpose!=='unsigned-tester-prerelease'||stage.target!==target||stage.workspaceDirty) throw new Error('Unsigned tester provenance mismatch');
  const report=await inspectPackage(app,stage.entries,process.platform==='darwin'?async path=>{
   const expected=stage.entries.find(e=>path.endsWith('/'+e.path));
   run('codesign',['--verify','--strict',path]);
   if(!expected||await normalizedMac(path)!==await normalizedMac(resolve('src-tauri/release-assets',expected.path))) throw new Error('Executable changed beyond ad-hoc signature');
  }:undefined);
  for(const entry of report.entries.filter(e=>/(^|\/)(media\/(ffmpeg|ffprobe)(\.exe)?|mcp\/soundshelf-mcp(\.exe)?)$/.test(e.path))) binaryArchitecture(await readFile(resolve(app,entry.path)),target);
  if(process.platform==='darwin') run('codesign',['--verify','--deep','--strict',app]);
  const bridge=report.entries.find(e=>e.path.endsWith('/mcp/soundshelf-mcp'+(process.platform==='win32'?'.exe':''))||e.path==='mcp/soundshelf-mcp'+(process.platform==='win32'?'.exe':''));
  if(bridge) {
   const smoke=spawnSync(resolve(app,bridge.path),[],{encoding:'utf8',timeout:10000});
   if(smoke.error||smoke.status!==1||smoke.stdout||!smoke.stderr.startsWith('CreativeShelf MCP bridge unavailable.')) throw new Error('Packaged MCP bridge did not load and reject missing arguments safely');
   report.bridgeStartup='Extracted executable loaded and rejected missing arguments; no endpoint or credential supplied';
  }

  const data=await readFile(artifact);
  report.artifact={name:artifact.split(/[\\/]/).pop(),bytes:data.length,sha256:sha(data)};
  Object.assign(report,{target,commit:stage.commit,purpose:stage.purpose,signature:{verified:false,method:process.platform==='darwin'?'ad-hoc only; no Developer ID or notarization':'unsigned'},installedQualification:'Automated content checks only; physical audio, live clients and long-session qualification pending'});
  await mkdir('release-reports/unsigned',{recursive:true});
  await writeFile(`release-reports/unsigned/${report.artifact.name}.json`,JSON.stringify(report,null,2));
 } else throw new Error('Use unsigned-release.mjs build|inspect EXTRACTED_APP INSTALLER');
} catch(e) { console.error(e.message); process.exitCode=1; }
