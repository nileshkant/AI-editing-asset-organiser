import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { App } from './App';
import { duration } from './api';
import type { Sound, SearchResults, PlaybackStatus } from './types';

const mockInvoke = vi.fn();
let tauriEnabled = false;

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => tauriEnabled,
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock('@tauri-apps/api/webviewWindow', () => ({
  getCurrentWebviewWindow: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));

const SOUND_A: Sound = {
  id: 'sound-a',
  source_id: 'src-1',
  relative_path: 'audio/whoosh.wav',
  title: 'Cinematic Whoosh Stereo',
  content_hash: 'hash-a',
  status: 'ready',
  profile: {
    duration: 1.5,
    sample_rate: 48000,
    channels: 2,
    channel_layout: 'stereo',
    channel_peaks: [0.8, 0.8],
    channel_rms: [0.2, 0.2],
    frames: 72000,
    peak: 0.8,
    rms: 0.2,
    description: 'Fast stereo whoosh',
    tags: ['whoosh', 'cinematic'],
    waveform: [[0.1, 0.3], [0.2, 0.5], [-0.1, 0.4]],
  },
  user_tags: [],
  comment: '',
  favorite: false,
};

const SOUND_B: Sound = {
  id: 'sound-b',
  source_id: 'src-1',
  relative_path: 'audio/impact.wav',
  title: 'Heavy Impact Mono',
  content_hash: 'hash-b',
  status: 'ready',
  profile: {
    duration: 0.75,
    sample_rate: 44100,
    channels: 1,
    frames: 33075,
    peak: 0.95,
    rms: 0.6,
    description: 'Low sub impact',
    tags: ['impact', 'sub'],
    waveform: [[-0.9, 0.9], [-0.5, 0.5]],
  },
  user_tags: ['bass'],
  comment: 'Great for trailers',
  favorite: true,
};

const MOCK_RESULTS: SearchResults = {
  items: [SOUND_A, SOUND_B],
  total: 2,
  interpretation: {
    terms: [],
    excluded: [],
    min_duration: null,
    max_duration: null,
    corrected: [],
  },
};

const STOPPED_PLAYBACK: PlaybackStatus = {
  sound_id: null,
  state: 'stopped',
  position_seconds: 0,
  duration_seconds: 0,
  volume: 1,
  peak: 0,
  error: null,
};

beforeEach(() => {
  vi.clearAllMocks();
  tauriEnabled = true;
  mockInvoke.mockImplementation((cmd: string) => {
    switch (cmd) {
      case 'app_info':
        return Promise.resolve({
          version: '0.1.0',
          data_directory: '/data',
          desktop: true,
          media_tools: true,
        });
      case 'sources':
        return Promise.resolve([]);
      case 'jobs':
        return Promise.resolve([]);
      case 'search_sounds':
        return Promise.resolve(MOCK_RESULTS);
      case 'playback_status':
        return Promise.resolve(STOPPED_PLAYBACK);
      case 'get_sound':
        return Promise.resolve(SOUND_A);
      case 'playback_play':
      case 'playback_pause':
      case 'playback_resume':
      case 'playback_stop':
      case 'playback_seek':
      case 'playback_set_volume':
      case 'annotate':
        return Promise.resolve();
      default:
        return Promise.resolve(null);
    }
  });
});

describe('App', () => {
  // ─── Basic Rendering ───
  it('renders the workspace label and header', async () => {
    render(<App />);
    await waitFor(() => expect(screen.getByText('YOUR AUDIO WORKSPACE')).toBeInTheDocument());
    expect(screen.getByRole('heading', { level: 1, name: 'Library' })).toBeInTheDocument();
  });

  it('renders the sidebar navigation items', async () => {
    render(<App />);
    await waitFor(() => {
      expect(screen.getByText('Favorites')).toBeInTheDocument();
      expect(screen.getByText('Imports')).toBeInTheDocument();
      expect(screen.getByText('Settings')).toBeInTheDocument();
    });
  });

  it('renders the SoundShelf brand', async () => {
    render(<App />);
    await waitFor(() => expect(screen.getByText('SoundShelf')).toBeInTheDocument());
  });

  // ─── Duration Formatting ───
  it('formats durations correctly', () => {
    expect(duration(0)).toBe('0:00.00');
    expect(duration(5.5)).toBe('0:05.50');
    expect(duration(65.3)).toBe('1:05.30');
  });

  // ─── Transport Controls ───
  it('renders transport play, stop, and volume controls', async () => {
    render(<App />);
    await waitFor(() => {
      expect(screen.getByLabelText('Play')).toBeInTheDocument();
      expect(screen.getByLabelText('Stop')).toBeInTheDocument();
      expect(screen.getByLabelText('Volume')).toBeInTheDocument();
    });
  });

  it('adjusts volume via the volume slider', async () => {
    render(<App />);
    const slider = await waitFor(() => screen.getByLabelText('Volume'));
    fireEvent.change(slider, { target: { value: '0.5' } });
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('playback_set_volume', { volume: 0.5 }),
    );
  });

  it('toggles mute on button click', async () => {
    render(<App />);
    const muteBtn = await waitFor(() => screen.getByLabelText('Mute'));
    fireEvent.click(muteBtn);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('playback_set_volume', { volume: 0 }),
    );
  });

  // ─── Keyboard Shortcuts ───
  it('toggles play on spacebar press', async () => {
    render(<App />);
    // Wait for results to load so togglePlay has items
    await waitFor(() => screen.getByText('Cinematic Whoosh Stereo'));
    fireEvent.keyDown(window, { code: 'Space', key: ' ' });
    // Should attempt to play (since nothing is playing, it will try the first result)
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('playback_play', expect.any(Object)),
    );
  });

  it('focuses search on "/" key', async () => {
    render(<App />);
    const searchInput = await waitFor(() => screen.getByLabelText('Search audio'));
    fireEvent.keyDown(window, { key: '/' });
    expect(document.activeElement).toBe(searchInput);
  });

  it('does not fire space play when typing in search', async () => {
    render(<App />);
    const searchInput = await waitFor(() => screen.getByLabelText('Search audio'));
    searchInput.focus();
    const callCountBefore = mockInvoke.mock.calls.filter(
      (c) => c[0] === 'playback_play',
    ).length;
    fireEvent.keyDown(searchInput, { code: 'Space', key: ' ' });
    const callCountAfter = mockInvoke.mock.calls.filter(
      (c) => c[0] === 'playback_play',
    ).length;
    expect(callCountAfter).toBe(callCountBefore);
  });

  it('toggles mute on "m" key press', async () => {
    render(<App />);
    await waitFor(() => screen.getByText('SoundShelf'));
    fireEvent.keyDown(window, { key: 'm' });
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('playback_set_volume', expect.any(Object)),
    );
  });

  // ─── Selection & Inspector ───
  it('selects a sound and opens inspector optimistically', async () => {
    render(<App />);
    await waitFor(() => screen.getByText('Cinematic Whoosh Stereo'));
    // Click to select
    const selectBtn = screen.getByLabelText(/Cinematic Whoosh Stereo, duration/);
    fireEvent.click(selectBtn);
    // Optimistic: inspector should appear immediately with the item data
    await waitFor(() => expect(screen.getByText('DETAILS')).toBeInTheDocument());
  });

  it('maintains playing sound when selecting a different sound', async () => {
    // Play sound A, then select sound B — transport should still show A
    mockInvoke.mockImplementation((cmd: string, args?: any) => {
      if (cmd === 'search_sounds') return Promise.resolve(MOCK_RESULTS);
      if (cmd === 'playback_status')
        return Promise.resolve({
          ...STOPPED_PLAYBACK,
          sound_id: 'sound-a',
          state: 'playing',
        });
      if (cmd === 'get_sound') {
        const id = args?.id;
        if (id === 'sound-b') return Promise.resolve(SOUND_B);
        return Promise.resolve(SOUND_A);
      }
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      return Promise.resolve(null);
    });
    render(<App />);
    await waitFor(() => screen.getByText('Cinematic Whoosh Stereo'));
  });

  // ─── Escape Key ───
  it('closes inspector on Escape', async () => {
    render(<App />);
    await waitFor(() => screen.getByText('Cinematic Whoosh Stereo'));
    const selectBtn = screen.getByLabelText(/Cinematic Whoosh Stereo, duration/);
    fireEvent.click(selectBtn);
    await waitFor(() => expect(screen.getByText('DETAILS')).toBeInTheDocument());
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() =>
      expect(screen.queryByText('DETAILS')).not.toBeInTheDocument(),
    );
  });

  // ─── Error Handling ───
  it('shows error banner when IPC command fails', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'search_sounds')
        return Promise.reject(new Error('Database locked'));
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      return Promise.resolve(null);
    });
    render(<App />);
    await waitFor(() =>
      expect(screen.getByRole('alert')).toBeInTheDocument(),
    );
  });

  it('dismisses error on X button click', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'search_sounds')
        return Promise.reject(new Error('Test error'));
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      return Promise.resolve(null);
    });
    render(<App />);
    const errorBanner = await waitFor(() => screen.getByRole('alert'));
    expect(errorBanner).toBeInTheDocument();
    const dismissBtn = screen.getByLabelText('Dismiss error');
    fireEvent.click(dismissBtn);
    await waitFor(() =>
      expect(screen.queryByRole('alert')).not.toBeInTheDocument(),
    );
  });

  // ─── Pagination ───
  it('disables Previous button on first page', async () => {
    render(<App />);
    const prevBtn = await waitFor(() => screen.getByLabelText('Previous page'));
    expect(prevBtn).toBeDisabled();
  });

  it('disables Next button when all results fit on one page', async () => {
    render(<App />);
    const nextBtn = await waitFor(() => screen.getByLabelText('Next page'));
    expect(nextBtn).toBeDisabled();
  });

  // ─── ARIA & Accessibility ───
  it('provides a live region for announcements', async () => {
    render(<App />);
    const liveRegion = await waitFor(() =>
      screen.getByLabelText('Announcements'),
    );
    expect(liveRegion).toHaveAttribute('aria-live', 'polite');
    expect(liveRegion).toHaveAttribute('aria-atomic', 'true');
  });

  it('sound list uses listbox role', async () => {
    render(<App />);
    const listbox = await waitFor(() => screen.getByRole('listbox'));
    expect(listbox).toHaveAttribute('aria-label', 'Sounds');
  });

  it('seek slider has proper ARIA attributes', async () => {
    render(<App />);
    const slider = await waitFor(() => screen.getByLabelText('Seek position'));
    expect(slider).toHaveAttribute('aria-valuemin', '0');
    expect(slider).toHaveAttribute('aria-valuenow');
  });

  it('level meter has meter role', async () => {
    render(<App />);
    const meter = await waitFor(() => screen.getByRole('meter'));
    expect(meter).toHaveAttribute('aria-valuemin', '0');
    expect(meter).toHaveAttribute('aria-valuemax', '100');
  });

  // ─── Empty State ───
  it('shows empty state when no results', async () => {
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'search_sounds')
        return Promise.resolve({ ...MOCK_RESULTS, items: [], total: 0 });
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      return Promise.resolve(null);
    });
    render(<App />);
    await waitFor(() =>
      expect(screen.getByText('No sounds yet')).toBeInTheDocument(),
    );
  });

  // ─── Long Title Safety ───
  it('renders ultra-long sound titles without crash', async () => {
    const longTitle = 'A'.repeat(500);
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'search_sounds')
        return Promise.resolve({
          ...MOCK_RESULTS,
          items: [{ ...SOUND_A, title: longTitle }],
          total: 1,
        });
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      return Promise.resolve(null);
    });
    render(<App />);
    await waitFor(() => expect(screen.getByText(longTitle)).toBeInTheDocument());
  });

  // ─── Page Navigation ───
  it('navigates to imports page', async () => {
    render(<App />);
    await waitFor(() => screen.getByText('SoundShelf'));
    const importsBtn = screen.getByText('Imports');
    fireEvent.click(importsBtn);
    await waitFor(() =>
      expect(screen.getByText('No imports')).toBeInTheDocument(),
    );
  });

  it('navigates to settings page', async () => {
    render(<App />);
    await waitFor(() => screen.getByText('SoundShelf'));
    const settingsBtn = screen.getByText('Settings');
    fireEvent.click(settingsBtn);
    await waitFor(() =>
      expect(screen.getByText('Application')).toBeInTheDocument(),
    );
  });

  // ─── Core UX Regression Verifications ───
  it('prevents multiple playback_play calls when play is in-flight (rapid click debounce)', async () => {
    let playResolve: () => void = () => {};
    const playPromise = new Promise<void>((resolve) => {
      playResolve = resolve;
    });

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'search_sounds') return Promise.resolve(MOCK_RESULTS);
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      if (cmd === 'playback_play') return playPromise;
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      return Promise.resolve(null);
    });

    render(<App />);
    await waitFor(() => screen.getByText('Cinematic Whoosh Stereo'));

    const playButtons = screen.getAllByRole('button', { name: /Play Cinematic Whoosh Stereo/i });
    const rowPlayBtn = playButtons[0];

    // First click initiates play
    fireEvent.click(rowPlayBtn);

    // Second click immediately while play is still in-flight
    fireEvent.click(rowPlayBtn);

    const playCalls = mockInvoke.mock.calls.filter((c) => c[0] === 'playback_play');
    expect(playCalls.length).toBe(1);

    // Resolve play promise
    playResolve();
  });

  it('filters library by folder when selecting a source in sidebar', async () => {
    const mockSource = {
      id: 'src-folder-1',
      name: 'Foley Effects',
      root: '/audio/foley',
      generation: 1,
      available: true,
    };

    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'sources') return Promise.resolve([mockSource]);
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'search_sounds') return Promise.resolve(MOCK_RESULTS);
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      return Promise.resolve(null);
    });

    render(<App />);
    await waitFor(() => screen.getByText('Foley Effects'));

    const folderButton = screen.getByRole('button', { name: /Foley Effects/i });
    fireEvent.click(folderButton);

    await waitFor(() => {
      const searchCalls = mockInvoke.mock.calls.filter((c) => c[0] === 'search_sounds');
      const hasFilteredCall = searchCalls.some((c) => {
        const query = (c[1] as any)?.query;
        return query && Array.isArray(query.source_ids) && query.source_ids.includes('src-folder-1');
      });
      expect(hasFilteredCall).toBe(true);
    });
  });
});
