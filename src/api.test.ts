import { describe, it, expect, vi, beforeEach } from 'vitest';
import {
  duration,
  call,
  secondsToFrame,
  frameToSeconds,
  createClip,
  getClip,
  listClips,
  updateClip,
  rebindClip,
  deleteClip,
  playClip,
} from './api';

const mockInvoke = vi.fn();
let mockIsTauri = false;

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => mockIsTauri,
  invoke: (...args: unknown[]) => mockInvoke(...args),
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

describe('secondsToFrame() and frameToSeconds()', () => {
  it('converts seconds to frames at 48 kHz with exact rounding', () => {
    expect(secondsToFrame(0, 48000)).toBe('0');
    expect(secondsToFrame(1, 48000)).toBe('48000');
    expect(secondsToFrame(15, 48000)).toBe('720000');
    expect(secondsToFrame(0.5, 48000)).toBe('24000');
    // Fractional rounding: 1 / 48000 = ~0.000020833s
    expect(secondsToFrame(0.0000208333, 48000)).toBe('1');
  });

  it('converts seconds to frames at 44.1 kHz with exact rounding', () => {
    expect(secondsToFrame(0, 44100)).toBe('0');
    expect(secondsToFrame(1, 44100)).toBe('44100');
    expect(secondsToFrame(15, 44100)).toBe('661500');
    expect(secondsToFrame(0.5, 44100)).toBe('22050');
  });

  it('handles negative, NaN, and invalid sample rates safely in secondsToFrame', () => {
    expect(secondsToFrame(-5, 48000)).toBe('0');
    expect(secondsToFrame(NaN, 48000)).toBe('0');
    expect(secondsToFrame(1.0, 0)).toBe('0');
    expect(secondsToFrame(1.0, -48000)).toBe('0');
  });

  it('converts frames back to seconds at 48 kHz and 44.1 kHz', () => {
    expect(frameToSeconds('0', 48000)).toBe(0);
    expect(frameToSeconds('48000', 48000)).toBe(1);
    expect(frameToSeconds('720000', 48000)).toBe(15);
    expect(frameToSeconds(24000, 48000)).toBe(0.5);
    expect(frameToSeconds('44100', 44100)).toBe(1);
    expect(frameToSeconds('661500', 44100)).toBe(15);
  });

  it('handles invalid inputs safely in frameToSeconds', () => {
    expect(frameToSeconds('-100', 48000)).toBe(0);
    expect(frameToSeconds('invalid', 48000)).toBe(0);
    expect(frameToSeconds('48000', 0)).toBe(0);
    expect(frameToSeconds('48000', -48000)).toBe(0);
  });
});

describe('Clip API Tauri commands', () => {
  const sampleRecipe: import('./types').ClipRecipe = {
    asset_id: 'sound-abc',
    asset_version_id: 'hash-abc',
    source_sample_rate_hz: 48000,
    start_frame: '0',
    end_frame: '720000',
  };

  const sampleClip: import('./types').Clip = {
    id: 'clip-123',
    sound_id: 'sound-abc',
    name: 'Intro Clip',
    revision: 1,
    recipe: sampleRecipe,
    is_stale: false,
    stale_reason: null,
    created_at: 1700000000,
    updated_at: 1700000000,
  };

  beforeEach(() => {
    mockInvoke.mockReset();
    mockIsTauri = true;
  });

  it('calls create_clip with soundId, name, and recipe', async () => {
    mockInvoke.mockResolvedValueOnce(sampleClip);
    const result = await createClip('sound-abc', 'Intro Clip', sampleRecipe);
    expect(mockInvoke).toHaveBeenCalledWith('create_clip', {
      soundId: 'sound-abc',
      name: 'Intro Clip',
      recipe: sampleRecipe,
    });
    expect(result).toEqual(sampleClip);
  });

  it('calls get_clip with id', async () => {
    mockInvoke.mockResolvedValueOnce(sampleClip);
    const result = await getClip('clip-123');
    expect(mockInvoke).toHaveBeenCalledWith('get_clip', { id: 'clip-123' });
    expect(result).toEqual(sampleClip);
  });

  it('calls list_clips with soundId', async () => {
    mockInvoke.mockResolvedValueOnce([sampleClip]);
    const result = await listClips('sound-abc');
    expect(mockInvoke).toHaveBeenCalledWith('list_clips', { soundId: 'sound-abc' });
    expect(result).toEqual([sampleClip]);
  });

  it('calls update_clip with id, name, recipe, and expectedRevision', async () => {
    const updated = { ...sampleClip, revision: 2 };
    mockInvoke.mockResolvedValueOnce(updated);
    const result = await updateClip('clip-123', 'Updated Clip', sampleRecipe, 1);
    expect(mockInvoke).toHaveBeenCalledWith('update_clip', {
      id: 'clip-123',
      name: 'Updated Clip',
      recipe: sampleRecipe,
      expectedRevision: 1,
    });
    expect(result.revision).toBe(2);
  });

  it('calls rebind_clip with id', async () => {
    const rebound = { ...sampleClip, revision: 2, is_stale: false, stale_reason: null };
    mockInvoke.mockResolvedValueOnce(rebound);
    const result = await rebindClip('clip-123');
    expect(mockInvoke).toHaveBeenCalledWith('rebind_clip', { id: 'clip-123' });
    expect(result).toEqual(rebound);
  });

  it('calls delete_clip with id', async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await deleteClip('clip-123');
    expect(mockInvoke).toHaveBeenCalledWith('delete_clip', { id: 'clip-123' });
  });

  it('calls playback_play_clip with id and clipId', async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await playClip('sound-abc', 'clip-123');
    expect(mockInvoke).toHaveBeenCalledWith('playback_play_clip', {
      id: 'sound-abc',
      clipId: 'clip-123',
    });
  });
});


