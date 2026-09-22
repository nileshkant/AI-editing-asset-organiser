import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { SoundInspector } from './components/sound/SoundInspector';
import type { Sound, Clip } from './types';

const mockInvoke = vi.fn();
let mockIsTauri = false;

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => mockIsTauri,
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const SAMPLE_SOUND: Sound = {
  id: 'sound-inspect-1',
  source_id: 'src-1',
  relative_path: 'audio/cinematic.wav',
  title: 'Cinematic Hit',
  content_hash: 'hash-cinematic-v1',
  status: 'ready',
  profile: {
    duration: 10.0,
    sample_rate: 48000,
    channels: 2,
    channel_layout: 'stereo',
    channel_peaks: [0.9, 0.9],
    channel_rms: [0.3, 0.3],
    frames: 480000,
    peak: 0.9,
    rms: 0.3,
    description: 'Dramatic trailer hit',
    tags: ['cinematic', 'hit'],
    waveform: [[0.2, 0.4], [0.3, 0.6]],
  },
  user_tags: ['favorite'],
  comment: 'Good opener',
  favorite: false,
};

const FRESH_CLIP: Clip = {
  id: 'clip-1',
  sound_id: 'sound-inspect-1',
  name: 'Intro Transient',
  revision: 1,
  recipe: {
    asset_id: 'sound-inspect-1',
    asset_version_id: 'hash-cinematic-v1',
    source_sample_rate_hz: 48000,
    start_frame: '0',
    end_frame: '48000',
    channel_policy: 'preserve',
    gain_db: 0.0,
    fade_in_ms: 0,
    fade_out_ms: 0,
  },
  is_stale: false,
  stale_reason: null,
  created_at: 1700000000,
  updated_at: 1700000000,
};

const STALE_CLIP: Clip = {
  id: 'clip-2',
  sound_id: 'sound-inspect-1',
  name: 'Tail Stinger',
  revision: 1,
  recipe: {
    asset_id: 'sound-inspect-1',
    asset_version_id: 'hash-cinematic-v0-old',
    source_sample_rate_hz: 48000,
    start_frame: '48000',
    end_frame: '144000',
    channel_policy: 'preserve',
    gain_db: 0.0,
    fade_in_ms: 0,
    fade_out_ms: 0,
  },
  is_stale: true,
  stale_reason: 'Source asset content hash modified from hash-cinematic-v0-old to hash-cinematic-v1',
  created_at: 1700000000,
  updated_at: 1700000000,
};

describe('SoundInspector - Saved Clips & SS-012 features', () => {
  const onPlay = vi.fn().mockResolvedValue(undefined);
  const onSeek = vi.fn().mockResolvedValue(undefined);
  const onClose = vi.fn();
  const onSave = vi.fn().mockResolvedValue(undefined);
  const onError = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    mockIsTauri = true;
  });

  it('renders "No saved clips yet" when no clips exist', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_clips') return Promise.resolve([]);
      return Promise.resolve(null);
    });

    render(
      <SoundInspector
        sound={SAMPLE_SOUND}
        playback={null}
        onPlay={onPlay}
        onSeek={onSeek}
        onClose={onClose}
        onSave={onSave}
        onError={onError}
      />,
    );

    await waitFor(() => {
      expect(
        screen.getByText(/No clip variants saved yet/),
      ).toBeInTheDocument();
    });
    expect(screen.getByRole('heading', { name: 'Saved Clips' })).toBeInTheDocument();
    expect(screen.getByText('0')).toBeInTheDocument();
  });

  it('renders list of saved clips with revision and duration badges', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_clips') return Promise.resolve([FRESH_CLIP]);
      return Promise.resolve(null);
    });

    render(
      <SoundInspector
        sound={SAMPLE_SOUND}
        playback={null}
        onPlay={onPlay}
        onSeek={onSeek}
        onClose={onClose}
        onSave={onSave}
        onError={onError}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText('Intro Transient')).toBeInTheDocument();
      expect(screen.getByText('v1')).toBeInTheDocument();
      expect(screen.getByText('0:01.00')).toBeInTheDocument();
      expect(screen.getByText(/Frames: 0 – 48000/)).toBeInTheDocument();
    });
    expect(screen.getByRole('heading', { name: 'Saved Clips' })).toBeInTheDocument();
    expect(screen.getByText('1')).toBeInTheDocument();
  });

  it('renders stale warning badge and rebind button when clip is stale', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_clips') return Promise.resolve([STALE_CLIP]);
      return Promise.resolve(null);
    });

    render(
      <SoundInspector
        sound={SAMPLE_SOUND}
        playback={null}
        onPlay={onPlay}
        onSeek={onSeek}
        onClose={onClose}
        onSave={onSave}
        onError={onError}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText('Tail Stinger')).toBeInTheDocument();
      expect(screen.getByText('Source changed (stale)')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /Rebind/i })).toBeInTheDocument();
    });
  });

  it('clicking Rebind calls rebind_clip and clears stale state', async () => {
    const reboundClip: Clip = {
      ...STALE_CLIP,
      revision: 2,
      recipe: {
        ...STALE_CLIP.recipe,
        asset_version_id: 'hash-cinematic-v1',
      },
      is_stale: false,
      stale_reason: null,
    };

    mockInvoke.mockImplementation((cmd: string, args: any) => {
      if (cmd === 'list_clips') return Promise.resolve([STALE_CLIP]);
      if (cmd === 'rebind_clip' && args?.id === 'clip-2') {
        return Promise.resolve(reboundClip);
      }
      return Promise.resolve(null);
    });

    render(
      <SoundInspector
        sound={SAMPLE_SOUND}
        playback={null}
        onPlay={onPlay}
        onSeek={onSeek}
        onClose={onClose}
        onSave={onSave}
        onError={onError}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText('Source changed (stale)')).toBeInTheDocument();
    });

    const rebindBtn = screen.getByRole('button', { name: /Rebind/i });
    fireEvent.click(rebindBtn);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('rebind_clip', { id: 'clip-2' });
      expect(screen.queryByText('Source changed (stale)')).not.toBeInTheDocument();
      expect(screen.getByText('v2')).toBeInTheDocument();
    });
  });

  it('clicking Delete calls delete_clip and removes clip from view', async () => {
    mockInvoke.mockImplementation((cmd: string, args: any) => {
      if (cmd === 'list_clips') return Promise.resolve([FRESH_CLIP]);
      if (cmd === 'delete_clip' && args?.id === 'clip-1') {
        return Promise.resolve();
      }
      return Promise.resolve(null);
    });

    render(
      <SoundInspector
        sound={SAMPLE_SOUND}
        playback={null}
        onPlay={onPlay}
        onSeek={onSeek}
        onClose={onClose}
        onSave={onSave}
        onError={onError}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText('Intro Transient')).toBeInTheDocument();
    });

    const deleteBtn = screen.getByLabelText('Delete clip Intro Transient');
    fireEvent.click(deleteBtn);

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('delete_clip', { id: 'clip-1' });
      expect(screen.queryByText('Intro Transient')).not.toBeInTheDocument();
      expect(screen.getByText(/No clip variants saved yet/)).toBeInTheDocument();
    });
  });

  it('clicking Select loads clip boundaries into waveform selection', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_clips') return Promise.resolve([FRESH_CLIP]);
      return Promise.resolve(null);
    });

    render(
      <SoundInspector
        sound={SAMPLE_SOUND}
        playback={null}
        onPlay={onPlay}
        onSeek={onSeek}
        onClose={onClose}
        onSave={onSave}
        onError={onError}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText('Intro Transient')).toBeInTheDocument();
    });

    const selectBtn = screen.getByRole('button', { name: 'Select' });
    fireEvent.click(selectBtn);

    // Waveform inputs should now show start=0.000 and end=1.000
    await waitFor(() => {
      const startInput = screen.getByLabelText('Selection start in seconds') as HTMLInputElement;
      const endInput = screen.getByLabelText('Selection end in seconds') as HTMLInputElement;
      expect(startInput.value).toBe('0.000');
      expect(endInput.value).toBe('1.000');
    });
  });

  it('clicking Play plays the sound and seeks to clip start', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'list_clips') return Promise.resolve([FRESH_CLIP]);
      return Promise.resolve(null);
    });

    render(
      <SoundInspector
        sound={SAMPLE_SOUND}
        playback={null}
        onPlay={onPlay}
        onSeek={onSeek}
        onClose={onClose}
        onSave={onSave}
        onError={onError}
      />,
    );

    await waitFor(() => {
      expect(screen.getByText('Intro Transient')).toBeInTheDocument();
    });

    const playBtn = screen.getByRole('button', { name: /Play/i });
    fireEvent.click(playBtn);

    await waitFor(() => {
      expect(onPlay).toHaveBeenCalledWith(SAMPLE_SOUND);
      expect(onSeek).toHaveBeenCalledWith(0);
    });
  });
});
