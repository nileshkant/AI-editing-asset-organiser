import { mkdtemp, mkdir, writeFile, rm, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { beforeEach, afterEach, it, expect } from 'vitest';
import { approvedMedia, binaryArchitecture, inspectPackage, inventory, sha, stageResources } from './release-assets.mjs';
let root;
beforeEach(async () => { root = await mkdtemp(join(tmpdir(), 'creativeshelf-release-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });
const target = 'aarch64-apple-darwin';
function macho(cpu = 0x100000c) { const b = Buffer.alloc(32, 0); b.writeUInt32LE(0xfeedfacf); b.writeUInt32LE(cpu, 4); b.writeUInt32LE(2, 12); return b; }
async function fixture() {
  const binary = macho(), notices = Buffer.from('LGPL fixture notice: synthetic, never runnable');
  const entries = {};
  for (const [name, data] of [['ffmpeg', binary], ['ffprobe', binary], ['notices', notices]]) {
    await writeFile(join(root, name), data); entries[name] = { path: name, sha256: sha(data), bytes: data.length };
  }
  return { schema: 'creativeshelf-release-media/v1', targets: { [target]: { ...entries, version: 'fixture', license: 'LGPL-2.1-or-later', configuration: '--disable-shared', sourceUrl: 'https://example.com/source', buildInstructions: 'Fixture only' } } };
}
it('blocks missing reviewed binaries instead of using developer PATH', async () => {
  await expect(approvedMedia({ schema: 'creativeshelf-release-media/v1', targets: {} }, target, root)).rejects.toThrow('No reviewed media lock');
});
it('rejects changed hashes, nonfree builds and mismatched architectures', async () => {
  const lock = await fixture();
  await writeFile(join(root, 'ffmpeg'), macho(0x1000007));
  await expect(approvedMedia(lock, target, root)).rejects.toThrow('checksum mismatch');
  lock.targets[target].configuration = '--enable-nonfree';
  await expect(approvedMedia(lock, target, root)).rejects.toThrow('LGPL');
  expect(() => binaryArchitecture(macho(0x1000007), target)).toThrow('target-specific');
  expect(() => binaryArchitecture(Buffer.from('MZ'), 'x86_64-pc-windows-msvc')).toThrow('PE');
});
it('rejects traversal and oversized input before staging', async () => {
  const lock = await fixture(); lock.targets[target].ffmpeg.path = '../escape';
  await expect(approvedMedia(lock, target, root)).rejects.toThrow('Invalid release input path');
  lock.targets[target].ffmpeg.path = 'ffmpeg'; lock.targets[target].ffmpeg.bytes++;
  await expect(approvedMedia(lock, target, root)).rejects.toThrow('checksum mismatch');
});
it('stages only five approved resources, never source-directory contents', async () => {
  const media = await approvedMedia(await fixture(), target, root);
  await writeFile(join(root, 'private.wav'), 'private');
  const stage = join(root, 'stage');
  const result = await stageResources(media, macho(), target, stage);
  expect(result.entries).toHaveLength(5);
  expect(result.entries.some(e => e.path.includes('private'))).toBe(false);
  expect(Object.values(result.resources)).toContain('mcp/soundshelf-mcp');
  expect((await inspectPackage(stage, result.entries)).uncompressedBytes).toBe(result.bytes);
  await expect(stageResources(media, macho(), target, stage)).rejects.toThrow();
});
it('detects missing resources, tampered notices and signed-byte differences', async () => {
  const media = await approvedMedia(await fixture(), target, root);
  const stage = join(root, 'stage'), result = await stageResources(media, macho(), target, stage);
  await writeFile(join(stage, 'mcp/soundshelf-mcp'), Buffer.concat([macho(), Buffer.from('signature fixture')]));
  await expect(inspectPackage(stage, result.entries)).rejects.toThrow('changed');
  let verified = 0;
  const report = await inspectPackage(stage, result.entries, async () => { verified++; });
  expect(verified).toBe(1); expect(report.entries.find(e => e.path === 'mcp/soundshelf-mcp').preSigningSha256).toBeTruthy();
  await writeFile(join(stage, 'notices/FFMPEG.txt'), 'tampered');
  await expect(inspectPackage(stage, result.entries, async () => {})).rejects.toThrow('changed');
  await rm(join(stage, 'notices/FFMPEG.txt'));
  await expect(inspectPackage(stage, result.entries, async () => {})).rejects.toThrow('missing');
});
it('detects renamed SQLite and WAV data and ordinary media extensions', async () => {
  for (const [name, data] of [['renamed.bin', Buffer.from('SQLite format 3\0fixture')], ['renamed.bin', Buffer.from('RIFFxxxxWAVEfixture')], ['source.mp3', Buffer.from('fixture')]]) {
    await writeFile(join(root, name), data);
    await expect(inventory(root)).rejects.toThrow('Source media/development data');
    await rm(join(root, name));
  }
});
it('does not follow package symlinks or approve linked input files', async () => {
  const lock = await fixture();
  // Unix symlink fixture: Windows developer accounts may not have symlink privilege.
  if (process.platform === 'win32') return;
  await symlink(join(root, 'ffprobe'), join(root, 'linked'));
  lock.targets[target].ffmpeg.path = 'linked';
  await expect(approvedMedia(lock, target, root)).rejects.toThrow('linked');
  await expect(inventory(root)).rejects.toThrow('Linked package member');
});
it('rejects foreign target even with otherwise approved bytes', async () => {
  await expect(approvedMedia(await fixture(), 'aarch64-unknown-linux-gnu', root)).rejects.toThrow('Unsupported');
});
