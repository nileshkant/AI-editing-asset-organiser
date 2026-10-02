import { mkdtemp, mkdir, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { beforeEach, afterEach, it, expect } from 'vitest';
import { hostTarget, sha } from './release-assets.mjs';
let root;
const script=resolve('scripts/unsigned-release.mjs');
beforeEach(async()=>{root=await mkdtemp(join(tmpdir(),'creativeshelf-preview-test-'));await mkdir(join(root,'release-reports'));await mkdir(join(root,'app'));await writeFile(join(root,'installer.bin'),'fixture');});
afterEach(async()=>{await rm(root,{recursive:true,force:true});});
async function inspect(stage, files={}) {
 await writeFile(join(root,'release-reports/stage.json'),JSON.stringify(stage));
 for(const [name,value] of Object.entries(files)){const path=join(root,'app',name);await mkdir(resolve(path,'..'),{recursive:true});await writeFile(path,value);}
 const r=spawnSync(process.execPath,[script,'inspect',join(root,'app'),join(root,'installer.bin')],{cwd:root,encoding:'utf8'});
 return {status:r.status,error:r.stderr};
}
function stage(){return {purpose:'unsigned-tester-prerelease',target:hostTarget(),workspaceDirty:false,entries:[]};}
it('cannot repurpose signed or local evidence as an unsigned public preview',async()=>{
 for(const purpose of ['signed-release-candidate','local-development-only']){
  const s=stage();s.purpose=purpose;expect(await inspect(s)).toEqual({status:1,error:expect.stringContaining('provenance mismatch')});
 }
});
it('rejects dirty and wrong-target preview evidence before writing a report',async()=>{
 for(const s of [{...stage(),workspaceDirty:true},{...stage(),target:'another-target'}])expect((await inspect(s)).status).toBe(1);
 await expect(readFile(join(root,'release-reports/unsigned/installer.bin.json'))).rejects.toThrow();
});
it('refuses to publish source audio or user catalogs hidden in an installer',async()=>{
 const r=await inspect(stage(),{'private.wav':'fixture'});expect(r.status).toBe(1);expect(r.error).toContain('Source media/development data');
});
it('rejects missing or altered license notices without a signature exemption',async()=>{
 const bytes=Buffer.from('expected LGPL notice'),s=stage();s.entries=[{path:'notices/FFMPEG.txt',bytes:bytes.length,sha256:sha(bytes)}];
 expect((await inspect(s)).error).toContain('missing');
 expect((await inspect(s,{'notices/FFMPEG.txt':'changed'})).error).toContain('changed');
});
