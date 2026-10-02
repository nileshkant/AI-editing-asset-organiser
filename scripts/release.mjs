import { readFile, writeFile, mkdir, lstat, readdir } from 'node:fs/promises';
import { resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { rustEnv } from './cargo.mjs';
import { hostTarget, readLock, approvedMedia, qualifyTools, stageResources, inspectPackage, inventory, sha, binaryArchitecture } from './release-assets.mjs';
import { signingSettings, signResource, verifyResource, verifyArtifact, signLinuxArtifact } from './release-signing.mjs';
function run(command, args) {
  const result = spawnSync(command, args, { env: rustEnv, stdio: 'inherit' });
  if (result.error || result.status !== 0) throw new Error('Release build command failed');
}
try {
  const action = process.argv[2];
  if (!['check', 'build', 'inspect', 'local', 'inspect-local'].includes(action)) throw new Error('Use release.mjs check|build|inspect|local|inspect-local; inspection requires extracted app directory and artifact path.');
  const target = hostTarget(), suffix = process.platform === 'win32' ? '.exe' : '';
  if (action === 'inspect' || action === 'inspect-local') {
    const stage = JSON.parse(await readFile('release-reports/stage.json', 'utf8'));
    if (stage.target !== target) throw new Error('Staged target does not match host');
    const local = action === 'inspect-local';
    if ((stage.purpose === 'local-development-only') !== local) throw new Error('Local and release artifact evidence must stay separate');
    const settings = local ? { publisher: null, method: 'unsigned local development only' } : signingSettings(), app = resolve(process.argv[3]), artifact = resolve(process.argv[4]);
    const stat = await lstat(artifact);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size > 1024 * 1024 * 1024) throw new Error('Invalid installer artifact');
    if (!local) verifyArtifact(artifact, app, settings);
    const report = await inspectPackage(app, stage.entries, async path => { binaryArchitecture(await readFile(path), target); verifyResource(path, settings); });
    const resourcePattern = /\/(media\/(ffmpeg|ffprobe)(\.exe)?|mcp\/soundshelf-mcp(\.exe)?)$/;
    for (const entry of report.entries.filter(e => resourcePattern.test('/' + e.path))) {
      const path = resolve(app, entry.path); binaryArchitecture(await readFile(path), target);
      if (!local && process.platform !== 'linux') verifyResource(path, settings);
    }
    if (!local && process.platform === 'win32') {
      const binaries = report.entries.filter(e => /(^|\/)soundshelf\.exe$/.test(e.path));
      if (binaries.length !== 1) throw new Error('Installed main executable missing or duplicated');
      verifyResource(resolve(app, binaries[0].path), settings);
    }
    const data = await readFile(artifact);
    report.artifact = { name: artifact.split(/[\\/]/).pop(), bytes: data.length, sha256: sha(data) };
    report.target = target; report.resourceBytes = stage.bytes;
    report.mediaToolBytes = stage.entries.filter(e => e.path.startsWith('media/')).reduce((n, e) => n + e.bytes, 0);
    report.modelPackBytes = 0;
    report.signature = { verified: !local, publisher: settings.publisher, method: settings.method };
    report.signatureQualification = local ? 'Not qualified for distribution' : 'Platform publisher signature verified; installed SS-031 qualification remains required';
    await writeFile(local ? 'release-reports/local-package.json' : 'release-reports/package.json', JSON.stringify(report, null, 2));
    console.log('Package inventory and size report saved; installed qualification and human release review remain required.');
  } else {
    let lock;
    try { lock = await readLock('.release-input/media-lock.json'); }
    catch (e) { if (e.code !== 'ENOENT') throw e; lock = await readLock('release/media-lock.json'); }
    if (lock.targets?.[target]?.sourceSha256) {
      const source = JSON.parse(await readFile('release/ffmpeg-source.json', 'utf8'));
      if (lock.targets[target].sourceSha256 !== source.sha256) throw new Error('Generated media does not match reviewed source lock');
    }
    const media = await approvedMedia(lock, target, resolve('.release-input'));
    qualifyTools(media, target);
    console.log(`Audited media checks passed for ${target}.`);
    if (action === 'build' || action === 'local') {
      const local = action === 'local';
      if (local && process.platform !== 'darwin') throw new Error('Isolated local app check currently uses this macOS workspace; distribution remains platform-qualified separately');
      const dirty = spawnSync('git', ['status', '--porcelain'], { encoding: 'utf8' });
      if (dirty.status !== 0 || (!local && dirty.stdout.trim())) throw new Error('Commit a clean candidate checkout before signing a release');
      const settings = local ? null : signingSettings();
      run(process.execPath, ['scripts/cargo.mjs', 'build', '-p', 'soundshelf-agent', '--bin', 'soundshelf-mcp', '--release', '--locked', '--target', target]);
      const bridge = await readFile(`target/${target}/release/soundshelf-mcp${suffix}`);
      const stageDirectory = resolve(local ? 'src-tauri/release-assets-local' : 'src-tauri/release-assets');
      const stage = await stageResources(media, bridge, target, stageDirectory);
      if (!local) for (const name of [`media/ffmpeg${suffix}`, `media/ffprobe${suffix}`, `mcp/soundshelf-mcp${suffix}`]) signResource(resolve(stageDirectory, name), settings);
      stage.entries = await inventory(stageDirectory); stage.bytes = stage.entries.reduce((n, e) => n + e.bytes, 0);
      await mkdir('release-reports', { recursive: true });
      const bundle = { resources: stage.resources, targets: process.platform === 'darwin' ? ['app', 'dmg'] : process.platform === 'win32' ? ['nsis', 'msi'] : ['appimage', 'deb'] };
      if (process.platform === 'win32') bundle.windows = { certificateThumbprint: settings.publisher, digestAlgorithm: 'sha256', timestampUrl: settings.timestamp, tsp: true, allowDowngrades: false, webviewInstallMode: { type: 'offlineInstaller' } };
      const config = { bundle };
      if (local) {
        const base = JSON.parse(await readFile('src-tauri/tauri.conf.json', 'utf8'));
        bundle.targets = ['app']; config.productName = 'CreativeShelf Local'; config.identifier = 'app.creativeshelf.qualification.local';
        config.app = { windows: base.app.windows.map(w => ({ ...w, title: 'CreativeShelf · Local qualification' })) };
      }
      await writeFile('src-tauri/tauri.staged.conf.json', JSON.stringify(config, null, 2));
      const commit = spawnSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' });
      if (commit.status !== 0) throw new Error('Release commit unavailable');
      await writeFile('release-reports/stage.json', JSON.stringify({ ...stage, resources: undefined, purpose: local ? 'local-development-only' : 'signed-release-candidate', workspaceDirty: Boolean(dirty.stdout.trim()), target, commit: commit.stdout.trim(), cargoLockSha256: sha(await readFile('Cargo.lock')), npmLockSha256: sha(await readFile('package-lock.json')) }, null, 2));
      run(process.execPath, ['scripts/desktop.mjs', 'build', '--target', target, '--config', 'src-tauri/tauri.staged.conf.json', ...(local ? ['--no-sign'] : [])]);
      if (process.platform === 'linux') {
        for (const directory of ['appimage', 'deb']) for (const name of await readdir(`target/${target}/release/bundle/${directory}`)) if (/\.(AppImage|deb)$/.test(name)) signLinuxArtifact(resolve(`target/${target}/release/bundle/${directory}`, name), settings);
      }
      console.log(local ? 'Isolated local app built. Inspect with release:inspect-local; this unsigned bundle is not qualified for distribution.' : 'Candidate built. Inspect signed contents, then run SS-031 installed qualification before distribution.');
    }
  }
} catch (e) { console.error(`CreativeShelf release blocked: ${e.message}`); process.exitCode = 1; }
