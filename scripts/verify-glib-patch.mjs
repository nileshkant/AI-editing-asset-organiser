import { readFile, readdir, lstat } from 'node:fs/promises';
import { join } from 'node:path';
import { sha } from './release-assets.mjs';
const provenance = JSON.parse(await readFile('vendor/glib-provenance.json', 'utf8'));
const actual = new Map();
async function walk(root, prefix = '') {
  for (const item of await readdir(root, { withFileTypes: true })) {
    const path = join(root, item.name), key = prefix + item.name;
    if (item.isDirectory()) await walk(path, key + '/');
    else {
      const info = await lstat(path);
      if (!info.isFile() || info.isSymbolicLink()) throw new Error('Unexpected linked vendor member');
      actual.set(key, sha(await readFile(path)));
    }
  }
}
try {
  await walk('vendor/glib');
  if (actual.size !== Object.keys(provenance.upstreamFiles).length) throw new Error('Unexpected vendor file count');
  for (const [path, digest] of Object.entries(provenance.upstreamFiles)) {
    if (actual.get(path) !== (path === provenance.changedFile ? provenance.patchedSha256 : digest)) throw new Error(`Unreviewed GLib vendor change: ${path}`);
  }
  const patched = await readFile(join('vendor/glib', provenance.changedFile), 'utf8');
  const original = patched.replace('let mut p: *mut libc::c_char = std::ptr::null_mut();', 'let p: *mut libc::c_char = std::ptr::null_mut();').replace('                &mut p,', '                &p,');
  if (sha(Buffer.from(original)) !== provenance.upstreamFiles[provenance.changedFile]) throw new Error('GLib patch exceeds reviewed two-line fix');
  console.log(`Verified ${actual.size} upstream GLib files; only reviewed mutable out-pointer fix differs.`);
} catch (e) { console.error(e.message); process.exitCode = 1; }
