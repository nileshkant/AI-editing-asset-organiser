import { readFile, writeFile, mkdir, lstat, copyFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { availableParallelism } from 'node:os';
import * as pgp from 'openpgp';
import { sha, hostTarget, binaryArchitecture } from './release-assets.mjs';
import { fileURLToPath } from 'node:url';
export const audioConfiguration = [
  '--disable-everything', '--disable-autodetect', '--disable-network', '--disable-doc',
  '--disable-debug', '--disable-shared', '--enable-static', '--disable-x86asm',
  '--enable-ffmpeg', '--enable-ffprobe', '--enable-avcodec', '--enable-avformat',
  '--enable-avutil', '--enable-avfilter', '--enable-swresample',
  '--enable-protocol=file,pipe', '--enable-demuxer=wav,mp3,flac,ogg,mov,aac,aiff,asf,caf',
  '--enable-parser=aac,aac_latm,flac,mpegaudio,opus,vorbis',
  '--enable-decoder=pcm_s16le,pcm_s24le,pcm_s32le,pcm_u8,pcm_f32le,pcm_f64le,pcm_s16be,pcm_s24be,pcm_s32be,mp3,flac,vorbis,opus,aac,alac,wmav1,wmav2,wmapro,wmalossless,adpcm_ima_wav',
  '--enable-encoder=pcm_s16le,pcm_s24le,pcm_s32le,pcm_f32le,flac',
  '--enable-muxer=wav,flac,pcm_f32le',
  '--enable-filter=atrim,asetpts,volume,afade,aresample,aformat,anull',
];
export async function verifySource(lock, bytes, armoredKey, armoredSignature) {
  if (bytes.length !== lock.bytes || sha(bytes) !== lock.sha256) throw new Error('Pinned FFmpeg source checksum mismatch');
  const key = await pgp.readKey({ armoredKey });
  if (lock.signingFingerprint !== 'FCF986EA15E6E293A5644F10B4322F04D67658D8' || key.getFingerprint().toUpperCase() !== lock.signingFingerprint) throw new Error('Unexpected FFmpeg signing key');
  const result = await pgp.verify({ message: await pgp.createMessage({ binary: new Uint8Array(bytes) }), signature: await pgp.readSignature({ armoredSignature }), verificationKeys: key });
  if (result.signatures.length !== 1) throw new Error('Unexpected FFmpeg release signature count');
  await result.signatures[0].verified;
}
function run(command, args, cwd, capture = false) {
  const result = spawnSync(command, args, { cwd, stdio: capture ? 'pipe' : 'inherit', encoding: 'utf8', env: { ...process.env }, maxBuffer: 4 * 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error('Static media build command failed');
  return result.stdout;
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const target = hostTarget(), lock = JSON.parse(await readFile('release/ffmpeg-source.json', 'utf8'));
    const input = resolve('.release-input'), source = resolve(input, 'ffmpeg-source', `ffmpeg-${lock.version}.tar.xz`);
    await mkdir(resolve(input, 'ffmpeg-source'), { recursive: true });
    try { await lstat(source); } catch (e) {
      if (e.code !== 'ENOENT') throw e;
      if (lock.url !== `https://ffmpeg.org/releases/ffmpeg-${lock.version}.tar.xz` || lock.bytes > 32 * 1024 * 1024) throw new Error('Invalid source lock');
      const response = await fetch(lock.url, { redirect: 'error', signal: AbortSignal.timeout(120000) });
      if (!response.ok) throw new Error('Official source download failed');
      const chunks = []; let bytes = 0;
      for await (const chunk of response.body) { bytes += chunk.length; if (bytes > lock.bytes) throw new Error('Oversized source archive'); chunks.push(chunk); }
      await writeFile(source, Buffer.concat(chunks), { flag: 'wx' });
    }
    const stat = await lstat(source);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size !== lock.bytes) throw new Error('Invalid source archive');
    await verifySource(lock, await readFile(source), await readFile('release/ffmpeg-public-key.asc', 'utf8'), await readFile('release/ffmpeg-source-signature.asc', 'utf8'));
    const names = run('tar', ['-tf', source], undefined, true).split('\n').filter(Boolean);
    if (names.some(name => !name.startsWith(`ffmpeg-${lock.version}/`) || name.split('/').includes('..'))) throw new Error('Unsafe source archive member');
    const build = resolve(input, `ffmpeg-build-${target}`); await mkdir(build);
    run('tar', ['-xf', source, '--strip-components=1', '-C', build]);
    const args = [...audioConfiguration];
    if (process.platform === 'darwin') args.push('--extra-cflags=-mmacosx-version-min=14.2', '--extra-ldflags=-mmacosx-version-min=14.2');
    if (process.platform === 'win32') args.push('--target-os=mingw32', '--extra-ldflags=-static', '--disable-pthreads', '--enable-w32threads');
    if (process.platform === 'linux') args.push('--extra-ldflags=-static');
    run('bash', ['configure', ...args], build);
    run('make', ['-j', String(Math.min(4, availableParallelism()))], build);
    const output = resolve(input, target); await mkdir(output);
    const suffix = process.platform === 'win32' ? '.exe' : '';
    const entries = {};
    for (const name of ['ffmpeg', 'ffprobe']) {
      const path = resolve(build, name + suffix), bytes = await readFile(path); binaryArchitecture(bytes, target);
      await copyFile(path, resolve(output, name + suffix)); entries[name] = { path: `${target}/${name}${suffix}`, bytes: bytes.length, sha256: sha(bytes) };
    }
    const versionOutput = run(resolve(output, 'ffmpeg' + suffix), ['-version'], undefined, true);
    if (!/^\s*E\s+f32le\s/m.test(run(resolve(output, 'ffmpeg' + suffix), ['-hide_banner', '-muxers'], undefined, true))) throw new Error('Required raw float PCM muxer missing');
    const configuration = versionOutput.split('\n').find(line => line.startsWith('configuration: '))?.slice(15);
    if (!configuration || /--enable-(gpl|nonfree|shared)\b/.test(configuration)) throw new Error('Unsafe media configuration');
    const licenseOutput = run(resolve(output, 'ffmpeg' + suffix), ['-L'], undefined, true);
    if (!licenseOutput.includes('version 2.1 of the License, or (at your option) any later version')) throw new Error('Unexpected static media license');
    const notice = Buffer.from(`CreativeShelf media runtime: FFmpeg ${lock.version}\nSource archive: ${lock.url}\nSHA-256: ${lock.sha256}\nSignature: ${lock.signingFingerprint}\nBuild: npm ci && npm run media:build (reviewed scripts/media-build.mjs)\nConfiguration: ${configuration}\nNo GPL/nonfree/shared dependencies enabled. Original FFmpeg source is unmodified.\nDistribute the exact source archive and this build script beside release downloads.\n\n${await readFile(resolve(build, 'COPYING.LGPLv2.1'), 'utf8')}`);
    await writeFile(resolve(output, 'FFMPEG.txt'), notice, { flag: 'wx' });
    entries.notices = { path: `${target}/FFMPEG.txt`, bytes: notice.length, sha256: sha(notice) };
    await writeFile(resolve(input, 'media-lock.json'), JSON.stringify({ schema: 'creativeshelf-release-media/v1', targets: { [target]: { ...entries, version: lock.version, license: 'LGPL-2.1-or-later', configuration, sourceUrl: lock.url, sourceSha256: lock.sha256, buildInstructions: 'npm ci && npm run media:build; pinned source signature verified by scripts/media-build.mjs' } } }, null, 2));
    console.log(JSON.stringify({ target, signatureVerified: true, sourceSha256: lock.sha256, ffmpegBytes: entries.ffmpeg.bytes, ffprobeBytes: entries.ffprobe.bytes }));
  } catch (e) { console.error(`Media preparation blocked: ${e.message}`); process.exitCode = 1; }
}
