// Native import-table checks: never load a DLL or execute an ELF to inspect closure.
function demand(value, message) { if (!value) throw new Error(message); }
export function windowsImports(data) {
  demand(data.length >= 64 && data.toString('ascii', 0, 2) === 'MZ', 'Invalid PE input');
  const pe = data.readUInt32LE(60);
  demand(pe >= 64 && pe + 24 <= data.length && data.readUInt32LE(pe) === 0x4550, 'Truncated PE header');
  const count = data.readUInt16LE(pe + 6), size = data.readUInt16LE(pe + 20), opt = pe + 24;
  demand(count > 0 && count <= 96 && size >= 240 && opt + size + count * 40 <= data.length && data.readUInt16LE(opt) === 0x20b, 'Invalid PE sections/optional header');
  const directories = data.readUInt32LE(opt + 108);
  demand(directories >= 14 && data.readUInt32LE(opt + 112 + 13 * 8) === 0, 'Delayed imports require separate runtime qualification');
  const headers = data.readUInt32LE(opt + 60);
  function offset(rva, bytes) {
    if (rva < headers && rva + bytes <= Math.min(headers, data.length)) return rva;
    for (let i = 0; i < count; i++) {
      const s = opt + size + i * 40, address = data.readUInt32LE(s + 12), rawSize = data.readUInt32LE(s + 16), raw = data.readUInt32LE(s + 20);
      if (rva >= address && rva - address + bytes <= rawSize && raw + rva - address + bytes <= data.length) return raw + rva - address;
    }
    throw new Error('PE import RVA outside bounded file');
  }
  const rva = data.readUInt32LE(opt + 120), tableSize = data.readUInt32LE(opt + 124);
  if (rva === 0 && tableSize === 0) return [];
  demand(rva > 0 && tableSize >= 20 && tableSize <= 20 * 257, 'Invalid PE import table');
  const names = [];
  for (let i = 0; i < Math.min(257, Math.floor(tableSize / 20)); i++) {
    const o = offset(rva + i * 20, 20);
    if (data.subarray(o, o + 20).every(b => b === 0)) return names;
    const address = data.readUInt32LE(o + 12); let name = '';
    for (let n = 0; n < 256; n++) { const byte = data[offset(address + n, 1)]; if (byte === 0) break; demand(byte >= 32 && byte < 127, 'Invalid DLL name'); name += String.fromCharCode(byte); }
    demand(/^[a-z0-9_.-]+\.dll$/i.test(name) && name.length < 255, 'Invalid DLL import name'); names.push(name.toLowerCase());
  }
  throw new Error('Unterminated PE import table');
}
export function verifyRuntimeClosure(data, target) {
  if (target.endsWith('windows-msvc')) {
    const allowed = new Set(['kernel32.dll', 'advapi32.dll', 'user32.dll', 'ws2_32.dll', 'ole32.dll', 'shell32.dll', 'bcrypt.dll', 'secur32.dll', 'crypt32.dll', 'ntdll.dll', 'gdi32.dll', 'winmm.dll', 'msvcrt.dll', 'ucrtbase.dll', 'oleaut32.dll', 'comdlg32.dll']);
    for (const dll of windowsImports(data)) demand(allowed.has(dll) || /^api-ms-win-(core|crt|security)-[a-z0-9-]+\.dll$/.test(dll), `Non-system Windows runtime dependency: ${dll}`);
  } else if (target.endsWith('linux-gnu')) {
    demand(data.length >= 64 && data.subarray(0, 4).toString('hex') === '7f454c46' && data[4] === 2 && data[5] === 1, 'Invalid ELF input');
    const start = Number(data.readBigUInt64LE(32)), size = data.readUInt16LE(54), count = data.readUInt16LE(56);
    demand(Number.isSafeInteger(start) && size === 56 && count > 0 && count <= 128 && start + size * count <= data.length, 'Invalid ELF program headers');
    for (let i = 0; i < count; i++) {
      const o = start + size * i, type = data.readUInt32LE(o);
      demand(type !== 3, 'Linux media tool requires an external ELF loader');
      if (type === 2) {
        const dynamic = Number(data.readBigUInt64LE(o + 8)), bytes = Number(data.readBigUInt64LE(o + 32));
        demand(Number.isSafeInteger(dynamic) && Number.isSafeInteger(bytes) && bytes <= 65536 && bytes % 16 === 0 && dynamic + bytes <= data.length, 'Invalid ELF dynamic table');
        for (let p = dynamic; p < dynamic + bytes; p += 16) { const tag = data.readBigInt64LE(p); if (tag === 0n) break; demand(tag !== 1n, 'Linux media tool has a shared library dependency'); }
      }
    }
  } else throw new Error('Runtime closure check is target-specific');
}
