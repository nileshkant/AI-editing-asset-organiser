import { describe, it, expect, vi } from 'vitest';
import { duration, call } from './api';

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: vi.fn(),
}));

describe('duration()', () => {
  it('formats seconds under a minute', () => {
    expect(duration(5.5)).toBe('0:05.50');
  });

  it('formats zero seconds', () => {
    expect(duration(0)).toBe('0:00.00');
  });

  it('formats minutes and seconds', () => {
    expect(duration(65.3)).toBe('1:05.30');
  });

  it('handles NaN input gracefully', () => {
    expect(duration(NaN)).toBe('0:00.00');
  });

  it('handles negative input as zero', () => {
    expect(duration(-5)).toBe('0:00.00');
  });

  it('handles very small fractions', () => {
    expect(duration(0.01)).toBe('0:00.01');
  });

  it('handles durations over 60 minutes', () => {
    // 3665 seconds = 1 hour, 1 minute, 5 seconds
    const result = duration(3665);
    expect(result).toContain('1:');
  });

  it('handles large durations', () => {
    // Should not crash on very long durations
    const result = duration(86400); // 24 hours
    expect(result).toBeDefined();
    expect(typeof result).toBe('string');
  });
});

describe('call()', () => {
  it('rejects when not running in Tauri context', async () => {
    await expect(call('some_command')).rejects.toThrow(
      'Open SoundShelf as a desktop application.',
    );
  });

  it('rejects with proper error message', async () => {
    try {
      await call('test');
      expect.unreachable('should have thrown');
    } catch (e) {
      expect(e).toBeInstanceOf(Error);
      expect((e as Error).message).toContain('desktop application');
    }
  });
});
