import { readFile, writeFile, mkdir, lstat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { rustEnv } from './cargo.mjs';
import { hostTarget, readLock, approvedMedia, qualifyTools, stageResources, inspectPackage, inventory, sha, binaryArchitecture } from './release-assets.mjs';

function run(command, args) {
  const result = spawnSync(command, args, { env: rustEnv, stdio: 'inherit' });
  if (result.error || result.status !== 0) throw new Error('Release build command failed');
}
try {
  const action = process.argv[2];
  if (!['check', 'build', 'inspect'].includes(action)) throw new Error('Use release.mjs check|build|inspect; inspect requires extracted app directory and installer path.');
  const target = hostTarget();
  if (action === 'inspect') {
    const stage = JSON.parse(await readFile('release-reports/stage.json', 'utf8'));
    if (stage.target !== target) throw new Error('Staged target does not match host');
    if (!/^[A-Z0-9]{10}$/.test(process.env.APPLE_TEAM_ID || '')) throw new Error('Expected publisher team is required for package signature verification');
    const app = resolve(process.argv[3]);
    function verifySignature(path) {
      for (const args of [['--verify', '--strict', '--verbose=2', path], ['-d', '--verbose=4', path]]) {
        const result = spawnSync('/usr/bin/codesign', args, { encoding: 'utf8', timeout: 15000 });
        if (result.status !== 0) throw new Error('Package signature verification failed');
        if (args[0] === '-d' && !(result.stdout + result.stderr).includes(`TeamIdentifier=${process.env.APPLE_TEAM_ID}\n`)) throw new Error('Unexpected package publisher');
      }
    }
    verifySignature(app);
    const gatekeeper = spawnSync('/usr/sbin/spctl', ['--assess', '--type', 'execute', '--verbose=2', app], { encoding: 'utf8', timeout: 30000 });
    if (gatekeeper.status !== 0) throw new Error('macOS Gatekeeper assessment failed');
    const report = await inspectPackage(app, stage.entries, async path => { binaryArchitecture(await readFile(path), target); verifySignature(path); });
    for (const entry of report.entries.filter(e => /\/(media\/(ffmpeg|ffprobe)|mcp\/soundshelf-mcp)$/.test('/' + e.path))) {
      const path = resolve(app, entry.path); binaryArchitecture(await readFile(path), target); verifySignature(path);
    }
    const artifact = resolve(process.argv[4]), stat = await lstat(artifact);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 1024 * 1024 * 1024) throw new Error('Invalid installer artifact');
    const data = await readFile(artifact);
    report.artifact = { name: artifact.split(/[\\/]/).pop(), bytes: data.length, sha256: sha(data) };
    report.target = target;
    report.resourceBytes = stage.bytes;
    report.mediaToolBytes = stage.entries.filter(e => e.path.startsWith('media/')).reduce((n, e) => n + e.bytes, 0);
    report.modelPackBytes = 0;
    report.signatureQualification = 'codesign publisher and Gatekeeper checks passed; installed SS-031 qualification still required';
    report.signature = { verified: true, publisher: process.env.APPLE_TEAM_ID, method: 'codesign and Gatekeeper' };
    await writeFile('release-reports/package.json', JSON.stringify(report, null, 2));
    console.log('Package inventory and size report saved; signature and installed qualification remain required.');
  } else {
    const media = await approvedMedia(await readLock('release/media-lock.json'), target, resolve('.release-input'));
    qualifyTools(media, target);
    console.log(`Audited media checks passed for ${target}.`);
    if (action === 'build') {
      if (!process.env.APPLE_SIGNING_IDENTITY || !process.env.APPLE_TEAM_ID || !process.env.APPLE_ID || !process.env.APPLE_PASSWORD) throw new Error('Signed/notarized macOS candidate requires signing identity and notarization credentials through private environment settings.');
      run(process.execPath, ['scripts/cargo.mjs', 'build', '-p', 'soundshelf-agent', '--bin', 'soundshelf-mcp', '--release', '--locked', '--target', target]);
      const bridge = await readFile(`target/${target}/release/soundshelf-mcp`);
      const stage = await stageResources(media, bridge, target, resolve('src-tauri/release-assets'));
      for (const name of ['media/ffmpeg', 'media/ffprobe', 'mcp/soundshelf-mcp']) {
        run('/usr/bin/codesign', ['--force', '--options', 'runtime', '--timestamp', '--sign', process.env.APPLE_SIGNING_IDENTITY, resolve('src-tauri/release-assets', name)]);
      }
      stage.entries = await inventory(resolve('src-tauri/release-assets'));
      stage.bytes = stage.entries.reduce((n, e) => n + e.bytes, 0);
      await mkdir('release-reports', { recursive: true });
      const config = { bundle: { resources: stage.resources, targets: ['app', 'dmg'] } };
      await writeFile('src-tauri/tauri.staged.conf.json', JSON.stringify(config, null, 2));
      const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
      if (commit.status !== 0) throw new Error('Release commit unavailable');
      await writeFile('release-reports/stage.json', JSON.stringify({ ...stage, resources: undefined, target, commit: commit.stdout.trim(), cargoLockSha256: sha(await readFile('Cargo.lock')), npmLockSha256: sha(await readFile('package-lock.json')) }, null, 2));
      run(process.execPath, ['scripts/desktop.mjs', 'build', '--target', target, '--config', 'src-tauri/tauri.staged.conf.json']);
      console.log('Candidate built. Inspect extracted signed contents, verify notarization, then run SS-031 qualification before distribution.');
    }
  }
} catch (e) { console.error(`CreativeShelf release blocked: ${e.message}`); process.exitCode = 1; }
