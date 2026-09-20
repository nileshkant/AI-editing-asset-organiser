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
});
