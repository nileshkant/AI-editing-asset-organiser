import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const windowsIconPath = resolve('src-tauri/icons/icon.ico');

describe('desktop icon resources', () => {
  it('includes a valid multi-image Windows icon', () => {
    const icon = readFileSync(windowsIconPath);

    expect(icon.length).toBeGreaterThan(6);
    expect(icon.readUInt16LE(0)).toBe(0);
    expect(icon.readUInt16LE(2)).toBe(1);
    expect(icon.readUInt16LE(4)).toBeGreaterThan(1);
  });

  // tauri.conf.json keeps bundle.icon empty, so each packaging script must set it.
  it.each(['scripts/release.mjs', 'scripts/unsigned-release.mjs'])('%s gives the packaged app an icon', script => {
    const source = readFileSync(resolve(script), 'utf8').replace(/\s+/g, '');
    expect(source).toContain("bundle.icon=process.platform==='win32'?['icons/icon.ico']:['icons/icon.png']");
  });
});
