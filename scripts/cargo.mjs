import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = process.cwd();
const local = resolve(root, '.devtools/cargo/bin');
export const rustEnv = existsSync(local) ? {
  ...process.env,
  CARGO_HOME: resolve(root, '.devtools/cargo'),
  RUSTUP_HOME: resolve(root, '.devtools/rustup'),
  PATH: `${local}${process.platform === 'win32' ? ';' : ':'}${process.env.PATH}`,
} : process.env;

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const child = spawn('cargo', process.argv.slice(2), { env: rustEnv, stdio: 'inherit' });
  child.on('error', error => { console.error(error.message); process.exitCode = 1; });
  child.on('exit', code => { process.exitCode = code ?? 1; });
}
