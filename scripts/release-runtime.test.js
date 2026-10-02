import { it, expect } from 'vitest';
import { windowsImports, verifyRuntimeClosure } from './release-runtime.mjs';
function pe(dll = 'kernel32.dll') {
  const b = Buffer.alloc(0x600); b.write('MZ'); b.writeUInt32LE(128, 60); b.writeUInt32LE(0x4550, 128); b.writeUInt16LE(1, 134); b.writeUInt16LE(240, 148);
  const o = 152; b.writeUInt16LE(0x20b, o); b.writeUInt32LE(0x200, o + 60); b.writeUInt32LE(16, o + 108); b.writeUInt32LE(0x1000, o + 120); b.writeUInt32LE(40, o + 124);
  const s = o + 240; b.writeUInt32LE(0x1000, s + 12); b.writeUInt32LE(0x400, s + 16); b.writeUInt32LE(0x200, s + 20); b.writeUInt32LE(0x1100, 0x20c); b.write(dll, 0x300); return b;
}
function elf(type = 1) { const b = Buffer.alloc(256); b.set([0x7f,69,76,70,2,1]); b.writeBigUInt64LE(64n, 32); b.writeUInt16LE(56, 54); b.writeUInt16LE(1, 56); b.writeUInt32LE(type, 64); return b; }
it('accepts Windows system imports and rejects redistributable or custom DLL dependencies', () => {
  expect(windowsImports(pe())).toEqual(['kernel32.dll']);
  expect(() => verifyRuntimeClosure(pe('psapi.dll'), 'x86_64-pc-windows-msvc')).not.toThrow(); expect(() => verifyRuntimeClosure(pe(), 'x86_64-pc-windows-msvc')).not.toThrow();
  expect(() => verifyRuntimeClosure(pe('avcodec-63.dll'), 'x86_64-pc-windows-msvc')).toThrow('Non-system');
  expect(() => verifyRuntimeClosure(pe('vcruntime140.dll'), 'x86_64-pc-windows-msvc')).toThrow('Non-system');
});
it('rejects forged PE RVAs and unqualified delayed imports before loading anything', () => {
  const b = pe(); b.writeUInt32LE(0xfffffff0, 0x20c); expect(() => windowsImports(b)).toThrow('outside');
  const delayed = pe(); delayed.writeUInt32LE(1, 152 + 112 + 13 * 8); expect(() => windowsImports(delayed)).toThrow('Delayed');
});
it('accepts static ELF and rejects external loader, shared dependency and truncated headers', () => {
  expect(() => verifyRuntimeClosure(elf(), 'x86_64-unknown-linux-gnu')).not.toThrow();
  expect(() => verifyRuntimeClosure(elf(3), 'x86_64-unknown-linux-gnu')).toThrow('external ELF loader');
  const dynamic = elf(2); dynamic.writeBigUInt64LE(128n, 72); dynamic.writeBigUInt64LE(32n, 96); dynamic.writeBigInt64LE(1n, 128);
  expect(() => verifyRuntimeClosure(dynamic, 'x86_64-unknown-linux-gnu')).toThrow('shared library');
  expect(() => verifyRuntimeClosure(elf().subarray(0, 80), 'x86_64-unknown-linux-gnu')).toThrow('program headers');
});

it('accepts a GNU-linked import directory containing extra thunk/name data but keeps bounded reads', () => {
  const b = Buffer.concat([pe(), Buffer.alloc(0x3000)]);
  b.writeUInt32LE(0x3000, 152 + 240 + 16); b.writeUInt32LE(0x2000, 152 + 124);
  expect(windowsImports(b)).toEqual(['kernel32.dll']);
  b.writeUInt32LE(0xffffffff, 152 + 124);
  expect(() => windowsImports(b)).toThrow('Invalid PE import table');
});
