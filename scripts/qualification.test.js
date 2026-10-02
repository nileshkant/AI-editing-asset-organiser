import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { beforeEach, afterEach, it, expect } from 'vitest';
import { qualifyCandidate, requestedTargets, requiredChecks } from './qualification.mjs';
import { sha } from './release-assets.mjs';
let root;
beforeEach(async () => { root = await mkdtemp(join(tmpdir(), 'creativeshelf-qualification-')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });
async function evidence(path, value) { const bytes = Buffer.from(typeof value === 'string' ? value : JSON.stringify(value)); await writeFile(join(root, path), bytes); return { path, sha256: sha(bytes) }; }
async function fixture() {
  const commit = 'a'.repeat(40), digest = 'b'.repeat(64), observation = await evidence('observations.txt', 'Synthetic test evidence, not a qualified application install.');
  const targets = [];
  for (const [i, target] of requestedTargets.entries()) {
    const stageReport = await evidence(`stage-${i}.json`, { commit, target, purpose: 'signed-release-candidate', workspaceDirty: false, cargoLockSha256: digest, npmLockSha256: digest });
    const packageReport = await evidence(`package-${i}.json`, { target, artifact: { sha256: digest, bytes: 1000 }, uncompressedBytes: 1200, resourceBytes: 800, mediaToolBytes: 600, modelPackBytes: 0, entries: [{ path: 'fixture', bytes: 1200, sha256: digest }], signature: { verified: true, publisher: 'Fixture publisher' } });
    targets.push({ target, os: 'Fixture OS', minimumOs: 'Fixture version', tester: 'Automated fixture only', testDate: '2026-10-02', stageReport, packageReport, checks: requiredChecks.map(id => ({ id, result: 'pass', observed: 'Synthetic pass used only to test validator behavior.', evidence: observation })), defects: [] });
  }
  return { schema: 'creativeshelf-qualification/v1', commit, targets };
}
it('accepts internally complete evidence while reserving the human release decision', async () => {
  const result = await qualifyCandidate(await fixture(), root);
  expect(result.evidenceComplete).toBe(true); expect(result.meaning).toContain('human release decision');
});
it('cannot count a partial platform matrix as the requested release', async () => {
  const manifest = await fixture(); manifest.targets.pop();
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('All requested');
});
it('rejects evidence for a different artifact commit or target', async () => {
  const manifest = await fixture(); manifest.commit = 'c'.repeat(40);
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('another commit');
  manifest.commit = 'a'.repeat(40); manifest.targets[0].stageReport = manifest.targets[1].stageReport;
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('another commit or target');
});
it('requires real observed outcomes and all distinct checks', async () => {
  const manifest = await fixture(); manifest.targets[0].checks[0].result = 'pending';
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('pending/failed');
  manifest.targets[0].checks[0].result = 'pass'; manifest.targets[0].checks[1].id = manifest.targets[0].checks[0].id;
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('duplicate');
});
it('rejects tampered evidence and traversal', async () => {
  const manifest = await fixture(); await writeFile(join(root, 'observations.txt'), 'Changed');
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('checksum mismatch');
  manifest.targets[0].checks[0].evidence.path = '../outside';
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('Invalid evidence path');
});
it('blocks high defects and medium defects without workarounds', async () => {
  const manifest = await fixture(); manifest.targets[0].defects = [{ severity: 'high', workaround: 'fixture' }];
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('high/critical');
  manifest.targets[0].defects[0] = { severity: 'medium', workaround: '' };
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('undocumented defect');
});
it('blocks an unsigned artifact even if all human checks say pass', async () => {
  const manifest = await fixture(); manifest.targets[0].packageReport = await evidence('unsigned.json', { target: requestedTargets[0], artifact: { sha256: 'b'.repeat(64), bytes: 1000 }, uncompressedBytes: 1200, resourceBytes: 800, mediaToolBytes: 600, modelPackBytes: 0, entries: ['fixture'], signature: { verified: false, publisher: 'Fixture' } });
  await expect(qualifyCandidate(manifest, root)).rejects.toThrow('signature evidence');
});

it('blocks local or dirty stage evidence even when a signature report says pass', async () => {
  const manifest = await fixture();
  for (const settings of [{ purpose: 'local-development-only', workspaceDirty: false }, { purpose: 'signed-release-candidate', workspaceDirty: true }]) {
    manifest.targets[0].stageReport = await evidence('local-stage.json', { commit: manifest.commit, target: requestedTargets[0], cargoLockSha256: 'b'.repeat(64), npmLockSha256: 'b'.repeat(64), ...settings });
    await expect(qualifyCandidate(manifest, root)).rejects.toThrow('Clean signed release provenance');
  }
});
