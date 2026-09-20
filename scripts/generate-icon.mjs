import { mkdirSync, writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

// Deterministic raster brand mark; never used to represent an audio waveform.
const size = 128;
const pixels = Buffer.alloc((size * 4 + 1) * size);
const bars = [28, 52, 78, 44, 64];
for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
  const i = y * (size * 4 + 1) + 1 + x * 4;
  const bar = Math.floor((x - 25) / 16);
  const filled = bar >= 0 && bar < 5 && (x - 25) % 16 < 10 && Math.abs(y - 64) < bars[bar] / 2;
  pixels.set(filled ? [184, 245, 106, 255] : [21, 21, 25, 255], i);
}
function chunk(type, data) {
  const text = Buffer.from(type), bytes = Buffer.concat([text, data]);
  let crc = 0xffffffff;
  for (const byte of bytes) { crc ^= byte; for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1)); }
  const head = Buffer.alloc(4), tail = Buffer.alloc(4);
  head.writeUInt32BE(data.length); tail.writeUInt32BE((crc ^ 0xffffffff) >>> 0);
  return Buffer.concat([head, bytes, tail]);
}
const header = Buffer.alloc(13);
header.writeUInt32BE(size); header.writeUInt32BE(size, 4); header[8] = 8; header[9] = 6;
mkdirSync('src-tauri/icons', { recursive: true });
writeFileSync('src-tauri/icons/icon.png', Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]), chunk('IHDR',header), chunk('IDAT',deflateSync(pixels)), chunk('IEND',Buffer.alloc(0))]));
