import { spawn } from 'node:child_process';
import { rustEnv } from './cargo.mjs';
import { fileURLToPath } from 'node:url';

const cli = new URL('../node_modules/@tauri-apps/cli/tauri.js', import.meta.url);
const args = process.argv.slice(2);
const child = spawn(process.execPath, [fileURLToPath(cli), ...(args.length ? args : ['dev'])], {
  env: rustEnv, stdio: 'inherit',
});
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', code => { process.exitCode = code ?? 1; });
