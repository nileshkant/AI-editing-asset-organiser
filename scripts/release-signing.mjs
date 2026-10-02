import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
function run(command, args) {
  const r = spawnSync(command, args, { encoding: 'utf8', timeout: 120000, windowsHide: true });
  if (r.error || r.status !== 0) throw new Error('Platform signing or verification failed');
  return r.stdout + r.stderr;
}
export function signingSettings() {
  if (process.platform === 'darwin') {
    const publisher = process.env.APPLE_TEAM_ID;
    if (!/^[A-Z0-9]{10}$/.test(publisher || '') || !process.env.APPLE_SIGNING_IDENTITY || !process.env.APPLE_ID || !process.env.APPLE_PASSWORD) throw new Error('Apple signing identity, team and private notarization settings are required');
    return { publisher, method: 'codesign and Gatekeeper' };
  }
  if (process.platform === 'win32') {
    const publisher = process.env.WINDOWS_CERT_THUMBPRINT, timestamp = process.env.WINDOWS_TIMESTAMP_URL;
    if (!/^[A-Fa-f0-9]{40}$/.test(publisher || '')) throw new Error('Installed Windows signing certificate thumbprint required');
    const url = new URL(timestamp || '');
    if (url.protocol !== 'https:' || url.username || url.password) throw new Error('HTTPS timestamp service required');
    return { publisher: publisher.toUpperCase(), timestamp: url.toString(), method: 'Authenticode publisher certificate' };
  }
  const publisher = process.env.LINUX_SIGNING_FINGERPRINT;
  if (!/^(?:[A-Fa-f0-9]{40}|[A-Fa-f0-9]{64})$/.test(publisher || '')) throw new Error('Private Linux release signing key fingerprint required');
  return { publisher: publisher.toUpperCase(), method: 'detached GPG release signature' };
}
export function signResource(path, settings) {
  if (process.platform === 'darwin') run('/usr/bin/codesign', ['--force', '--options', 'runtime', '--timestamp', '--sign', process.env.APPLE_SIGNING_IDENTITY, path]);
  else if (process.platform === 'win32') run('signtool', ['sign', '/sha1', settings.publisher, '/fd', 'SHA256', '/tr', settings.timestamp, '/td', 'SHA256', path]);
}
export function verifyResource(path, settings) {
  if (process.platform === 'darwin') {
    run('/usr/bin/codesign', ['--verify', '--strict', '--verbose=2', path]);
    const output = run('/usr/bin/codesign', ['-d', '--verbose=4', path]);
    if (!output.includes(`TeamIdentifier=${settings.publisher}\n`)) throw new Error('Unexpected macOS publisher');
  } else if (process.platform === 'win32') run('powershell.exe', ['-NoProfile', '-NonInteractive', '-File', resolve('scripts/verify-windows-signature.ps1'), '-Artifact', path, '-ExpectedThumbprint', settings.publisher]);
  else throw new Error('Linux resources must retain exact content hashes');
}
export function verifyArtifact(path, app, settings) {
  if (process.platform === 'darwin') {
    verifyResource(app, settings); verifyResource(path, settings);
    run('/usr/sbin/spctl', ['--assess', '--type', 'execute', '--verbose=2', app]);
    run('/usr/bin/xcrun', ['stapler', 'validate', app]);
  } else if (process.platform === 'win32') verifyResource(path, settings);
  else {
    const output = run('gpg', ['--batch', '--status-fd', '1', '--verify', path + '.asc', path]);
    if (!output.split('\n').some(line => line.startsWith(`[GNUPG:] VALIDSIG ${settings.publisher} `))) throw new Error('Unexpected Linux release publisher');
  }
}
export function signLinuxArtifact(path, settings) { run('gpg', ['--batch', '--local-user', settings.publisher, '--armor', '--detach-sign', path]); }
