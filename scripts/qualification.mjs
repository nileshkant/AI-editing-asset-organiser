import { readFile, lstat, realpath } from 'node:fs/promises';
import { resolve, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { sha } from './release-assets.mjs';
export const requiredChecks = ['clean-offline-install', 'source-reference-reuse-prune-relink', 'annotations-search', 'physical-playback-devices', 'independent-editor-export', 'installed-mcp-scopes-reconnect-revoke', 'backup-restore-upgrade', 'interrupt-disk-full-recovery', 'uninstall-source-retention', 'security-advisories', 'accessibility', 'long-session-resource-performance', 'package-content-size-signature', 'tester-instructions-feedback'];
export const requestedTargets = ['aarch64-apple-darwin', 'x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu'];
function requireTrue(value, message) { if (!value) throw new Error(message); }
async function proof(root, ref) {
  requireTrue(ref && typeof ref.path === 'string' && /^[a-f0-9]{64}$/.test(ref.sha256), 'Missing evidence reference/checksum');
  requireTrue(!isAbsolute(ref.path) && !ref.path.includes('\\') && ref.path.split('/').every(p => p && p !== '.' && p !== '..'), 'Invalid evidence path');
  const full = resolve(root, ref.path), canonical = await realpath(full), rel = relative(root, canonical);
  requireTrue(rel && !rel.startsWith('..') && !isAbsolute(rel), 'Evidence escapes proof directory');
  const meta = await lstat(full); requireTrue(meta.isFile() && !meta.isSymbolicLink() && meta.size > 0 && meta.size <= 1024 * 1024, 'Evidence must be a bounded regular file');
  const bytes = await readFile(full); requireTrue(bytes.length === meta.size && sha(bytes) === ref.sha256, 'Evidence changed or checksum mismatch');
  return bytes;
}
export async function qualifyCandidate(manifest, directory) {
  requireTrue(manifest.schema === 'creativeshelf-qualification/v1', 'Unsupported qualification manifest');
  requireTrue(/^[a-f0-9]{40}$/.test(manifest.commit || ''), 'Exact candidate commit required');
  requireTrue(Array.isArray(manifest.targets) && manifest.targets.length === requestedTargets.length && new Set(manifest.targets.map(t => t.target)).size === requestedTargets.length && requestedTargets.every(t => manifest.targets.some(x => x.target === t)), 'All requested macOS/Windows/Linux target evidence is required');
  const root = await realpath(directory);
  for (const entry of manifest.targets) {
    requireTrue(typeof entry.os === 'string' && entry.os.trim() && typeof entry.minimumOs === 'string' && entry.minimumOs.trim(), 'Declared test OS and minimum version required');
    requireTrue(typeof entry.tester === 'string' && entry.tester.trim() && /^\d{4}-\d{2}-\d{2}$/.test(entry.testDate || ''), 'Human tester attribution and test date required');
    const artifact = JSON.parse(await proof(root, entry.packageReport));
    const stage = JSON.parse(await proof(root, entry.stageReport));
    requireTrue(stage.commit === manifest.commit && stage.target === entry.target && artifact.target === entry.target, 'Evidence belongs to another commit or target');
    requireTrue(/^[a-f0-9]{64}$/.test(stage.cargoLockSha256 || '') && /^[a-f0-9]{64}$/.test(stage.npmLockSha256 || ''), 'Dependency lock evidence required');
    requireTrue(/^[a-f0-9]{64}$/.test(artifact.artifact?.sha256 || '') && Number.isSafeInteger(artifact.artifact?.bytes) && artifact.artifact.bytes > 0 && Number.isSafeInteger(artifact.uncompressedBytes) && artifact.uncompressedBytes > 0 && artifact.resourceBytes > 0 && artifact.mediaToolBytes > 0 && artifact.modelPackBytes === 0 && Array.isArray(artifact.entries) && artifact.entries.length > 0, 'Exact artifact hash, inventory and sizes required');
    requireTrue(artifact.signature?.verified === true && typeof artifact.signature.publisher === 'string' && artifact.signature.publisher.trim(), 'Platform signature evidence required');
    requireTrue(Array.isArray(entry.checks) && entry.checks.length === requiredChecks.length && new Set(entry.checks.map(c => c.id)).size === requiredChecks.length, 'Missing or duplicate qualification checks');
    for (const id of requiredChecks) {
      const check = entry.checks.find(c => c.id === id);
      requireTrue(check?.result === 'pass' && typeof check.observed === 'string' && check.observed.trim().length >= 20, `Qualification pending/failed: ${entry.target}/${id}`);
      await proof(root, check.evidence);
    }
    requireTrue(Array.isArray(entry.defects) && entry.defects.every(d => ['low', 'medium'].includes(d.severity) && typeof d.workaround === 'string' && d.workaround.trim()), 'Open high/critical or undocumented defect blocks candidate');
  }
  return { evidenceComplete: true, commit: manifest.commit, targets: requestedTargets, meaning: 'Recorded evidence is internally complete; a human release decision is still required.' };
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    const path = resolve(process.argv[2] || 'release/qualification.json');
    const meta = await lstat(path); requireTrue(meta.isFile() && !meta.isSymbolicLink() && meta.size <= 1024 * 1024, 'Invalid qualification manifest');
    console.log(JSON.stringify(await qualifyCandidate(JSON.parse(await readFile(path, 'utf8')), resolve(process.argv[3] || 'release-reports')), null, 2));
  } catch (e) { console.error(`CreativeShelf tester release blocked: ${e.message}`); process.exitCode = 1; }
}
