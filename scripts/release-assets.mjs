import { createHash } from 'node:crypto';
import { lstat, readdir, readFile, realpath, mkdir, writeFile } from 'node:fs/promises';
import { resolve, relative, isAbsolute, dirname } from 'node:path';
import { spawnSync } from 'node:child_process';

export const targets = {
  'aarch64-apple-darwin': { platform: 'darwin', arch: 'arm64', format: 'macho', cpu: 0x100000c },
  'x86_64-apple-darwin': { platform: 'darwin', arch: 'x64', format: 'macho', cpu: 0x1000007 },
  'x86_64-pc-windows-msvc': { platform: 'win32', arch: 'x64', format: 'pe', cpu: 0x8664 },
};
const sha = data => createHash('sha256').update(data).digest('hex');
const fail = message => { throw new Error(message); };
export function hostTarget() {
  return Object.keys(targets).find(t => targets[t].platform === process.platform && targets[t].arch === process.arch)
    || fail('No release candidate target for this host. Linux qualification is blocked by SS-034.');
}
export function binaryArchitecture(data, target) {
  const spec = targets[target] || fail('Unsupported candidate target');
  if (spec.format === 'macho') {
    if (data.length < 32 || data.readUInt32LE(0) !== 0xfeedfacf || data.readUInt32LE(4) !== spec.cpu || data.readUInt32LE(12) !== 2) fail('Expected a target-specific 64-bit Mach-O executable');
  } else {
    if (data.length < 64 || data.toString('ascii', 0, 2) !== 'MZ') fail('Expected PE executable');
    const pe = data.readUInt32LE(60);
    if (pe < 64 || pe + 26 > data.length || data.readUInt32LE(pe) !== 0x4550 || data.readUInt16LE(pe + 4) !== spec.cpu || data.readUInt16LE(pe + 24) !== 0x20b) fail('Wrong or truncated PE architecture');
  }
}
async function boundedFile(path, limit) {
  const s = await lstat(path);
  if (!s.isFile() || s.isSymbolicLink() || s.size > limit || s.size === 0) fail('Missing, linked or oversized release file');
  const data = await readFile(path);
  if (data.length !== s.size || data.length > limit) fail('Release input changed while reading');
  return data;
}
function inside(root, path) {
  const rel = relative(root, path);
  if (!rel || rel.startsWith('..') || isAbsolute(rel)) fail('Release path escapes approved input');
}
async function lockedFile(root, entry, limit) {
  if (!entry || typeof entry.path !== 'string' || isAbsolute(entry.path) || entry.path.includes('\\') || entry.path.split('/').some(p => !p || p === '..' || p === '.')) fail('Invalid release input path');
  if (!/^[a-f0-9]{64}$/.test(entry.sha256) || !Number.isSafeInteger(entry.bytes) || entry.bytes < 1) fail('Missing reviewed checksum/size');
  const path = resolve(root, entry.path); inside(root, path); inside(root, await realpath(path));
  const data = await boundedFile(path, limit);
  if (data.length !== entry.bytes || sha(data) !== entry.sha256) fail(`Release checksum mismatch: ${entry.path}`);
  return { data, path };
}
export async function approvedMedia(lock, target, inputRoot) {
  if (lock.schema !== 'creativeshelf-release-media/v1' || !targets[target]) fail('Unsupported release lock/target');
  const entry = lock.targets?.[target];
  if (!entry) fail(`No reviewed media lock for ${target}. Audited media and notices must be supplied before packaging.`);
  if (!entry.version || !entry.buildInstructions || !entry.sourceUrl?.startsWith('https://') || entry.license !== 'LGPL-2.1-or-later' || typeof entry.configuration !== 'string' || /--enable-(gpl|nonfree|shared)\b/.test(entry.configuration)) fail('Missing static LGPL build provenance');
  const root = await realpath(inputRoot);
  const files = {};
  for (const name of ['ffmpeg', 'ffprobe']) {
    files[name] = await lockedFile(root, entry[name], 256 * 1024 * 1024);
    binaryArchitecture(files[name].data, target);
  }
  files.notices = await lockedFile(root, entry.notices, 4 * 1024 * 1024);
  // Build instructions and source URL are reviewed provenance, not an arbitrary shell command.
  return { entry, files };
}
function run(path, args) {
  const r = spawnSync(path, args, { encoding: 'utf8', timeout: 15000, maxBuffer: 1024 * 1024, windowsHide: true });
  if (r.error || r.status !== 0) fail('Audited binary qualification command failed');
  return r.stdout + r.stderr;
}
export function qualifyTools(media, target) {
  const spec = targets[target];
  if (process.platform !== spec.platform || process.arch !== spec.arch) fail('Media tools must be qualified on the native target');
  for (const name of ['ffmpeg', 'ffprobe']) {
    const output = run(media.files[name].path, ['-version']);
    if (!output.startsWith(`${name} version ${media.entry.version} `) || !output.includes(`configuration: ${media.entry.configuration}`) || /--enable-(gpl|nonfree|shared)\b/.test(output)) fail('Audited media version/configuration mismatch');
    if (spec.platform === 'darwin') {
      const dependencies = run('/usr/bin/otool', ['-L', media.files[name].path]).split('\n').slice(1).map(s => s.trim()).filter(Boolean);
      if (dependencies.some(d => !d.startsWith('/usr/lib/') && !d.startsWith('/System/Library/'))) fail('Media tools depend on non-system libraries');
    } else {
      // A static configure flag alone cannot prove a Windows runtime closure.
      fail('Windows static dependency/signature qualification is not yet available; candidate packaging is blocked.');
    }
  }
}
export async function inventory(root) {
  const entries = [];
  async function walk(path) {
    for (const item of await readdir(path, { withFileTypes: true })) {
      const full = resolve(path, item.name), name = relative(root, full).split('\\').join('/');
      if (item.isSymbolicLink()) fail(`Linked package member: ${name}`);
      if (item.isDirectory()) await walk(full);
      else if (item.isFile()) {
        const data = await boundedFile(full, 512 * 1024 * 1024);
        if (/\.(wav|mp3|flac|ogg|m4a|aac|aiff?|wma|caf|sqlite(?:-wal|-shm)?|db)$/i.test(name) || /(^|\/)(catalog\.json|node_modules|\.devtools|\.git)(\/|$)/i.test(name) || data.subarray(0, 16).toString() === 'SQLite format 3\0' || data.subarray(0, 4).toString() === 'fLaC' || (data.subarray(0, 4).toString() === 'RIFF' && data.subarray(8, 12).toString() === 'WAVE')) fail(`Source media/development data in package: ${name}`);
        entries.push({ path: name, bytes: data.length, sha256: sha(data) });
      } else fail(`Non-regular package member: ${name}`);
    }
  }
  await walk(root); return entries.sort((a, b) => a.path.localeCompare(b.path));
}
export async function stageResources(media, bridge, target, stage) {
  binaryArchitecture(bridge, target);
  // Stage is caller-owned and must be new; never reuse a directory containing stale inputs.
  await mkdir(stage, { recursive: false });
  const suffix = targets[target].platform === 'win32' ? '.exe' : '';
  const resources = {};
  for (const [path, data] of [
    [`media/ffmpeg${suffix}`, media.files.ffmpeg.data], [`media/ffprobe${suffix}`, media.files.ffprobe.data],
    [`mcp/soundshelf-mcp${suffix}`, bridge], ['notices/FFMPEG.txt', media.files.notices.data],
    ['notices/MEDIA-PROVENANCE.json', Buffer.from(JSON.stringify({ version: media.entry.version, license: media.entry.license, sourceUrl: media.entry.sourceUrl, buildInstructions: media.entry.buildInstructions, configuration: media.entry.configuration }, null, 2))],
  ]) {
    const full = resolve(stage, path); await mkdir(dirname(full), { recursive: true });
    await writeFile(full, data, { flag: 'wx', mode: path.startsWith('notices/') ? 0o644 : 0o755 });
    resources[full] = path;
  }
  const entries = await inventory(stage);
  return { resources, entries, bytes: entries.reduce((n, e) => n + e.bytes, 0) };
}
export async function inspectPackage(root, expectedResources, verifySignedBinary) {
  const entries = await inventory(root);
  for (const expected of expectedResources) {
    const matches = entries.filter(e => e.path === expected.path || e.path.endsWith(`/${expected.path}`));
    if (matches.length !== 1) fail(`Packaged resource missing or duplicated: ${expected.path}`);
    if (matches[0].sha256 !== expected.sha256) {
      // Code signing changes executable bytes. Only an independently verified publisher
      // signature can authorize that difference; text/provenance must match exactly.
      if (!/^(media\/(ffmpeg|ffprobe)|mcp\/soundshelf-mcp)$/.test(expected.path) || !verifySignedBinary) fail(`Packaged resource changed: ${expected.path}`);
      await verifySignedBinary(resolve(root, matches[0].path));
      matches[0].preSigningSha256 = expected.sha256;
    }
  }
  return { entries, uncompressedBytes: entries.reduce((n, e) => n + e.bytes, 0) };
}
export async function readLock(path) { return JSON.parse(await boundedFile(path, 1024 * 1024)); }
export { sha };
