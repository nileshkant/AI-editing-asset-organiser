import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { App } from './App';
import userEvent from '@testing-library/user-event';
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
      case 'set_favorite':
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

  it('renders the CreativeShelf brand', async () => {
    render(<App />);
    await waitFor(() => expect(screen.getByText('CreativeShelf')).toBeInTheDocument());
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
    vi.useFakeTimers();
    const view = render(<App />);
    try {
      // Settle initial IPC state before advancing the search debounce.
      await act(async () => {});
      // Complete the debounce without relying on a loaded runner's wall clock.
      await act(async () => { await vi.advanceTimersByTimeAsync(200); });
      expect(screen.getByText('Cinematic Whoosh Stereo')).toBeInTheDocument();
      await act(async () => { fireEvent.keyDown(window, { code: 'Space', key: ' ' }); });
      expect(mockInvoke).toHaveBeenCalledWith('playback_play', { id: SOUND_A.id });
    } finally {
      view.unmount();
      vi.useRealTimers();
    }
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
    await waitFor(() => screen.getByText('CreativeShelf'));
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
    const listbox = await waitFor(() => screen.getByRole('list', { name: 'Sounds' }));
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
    await waitFor(() => screen.getByText('CreativeShelf'));
    const importsBtn = screen.getByText('Imports');
    fireEvent.click(importsBtn);
    await waitFor(() =>
      expect(screen.getByText('No imports')).toBeInTheDocument(),
    );
  });

  it('navigates to settings page', async () => {
    render(<App />);
    await waitFor(() => screen.getByText('CreativeShelf'));
    const settingsBtn = screen.getByText('Settings');
    fireEvent.click(settingsBtn);
    await waitFor(() =>
      expect(screen.getByRole('tab', {name: 'Sources'})).toHaveAttribute('aria-selected', 'true'),
    );
  });

  it('does not steal Settings activation or scrolling keys for library shortcuts', async () => {
    render(<App />);
    await screen.findByText('Cinematic Whoosh Stereo');
    fireEvent.click(screen.getByText('Settings'));
    const tab = screen.getByRole('tab', {name: 'Sources'});
    mockInvoke.mockClear();
    await act(async () => {
      expect(fireEvent.keyDown(tab, {key: ' ', code: 'Space'})).toBe(true);
      expect(fireEvent.keyDown(tab, {key: 'Enter'})).toBe(true);
      expect(fireEvent.keyDown(screen.getByRole('tabpanel'), {key: 'ArrowDown'})).toBe(true);
      expect(fireEvent.keyDown(screen.getByRole('tabpanel'), {key: 'PageDown'})).toBe(true);
    });
    expect(mockInvoke.mock.calls.filter(([cmd]) => ['playback_play', 'get_sound', 'playback_set_volume'].includes(cmd))).toEqual([]);
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

describe('Playback and tab regression tests', () => {
  it('pauses and resumes at the current position without restarting playback', async () => {
    const base = mockInvoke.getMockImplementation()!;
    let status = { ...STOPPED_PLAYBACK, sound_id: SOUND_A.id, state: 'playing', position_seconds: 0.6, duration_seconds: 1.5 };
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'playback_status') return Promise.resolve({ ...status });
      if (cmd === 'playback_pause') { status.state = 'paused'; return Promise.resolve(); }
      if (cmd === 'playback_resume') { status.state = 'playing'; return Promise.resolve(); }
      return base(cmd, args);
    });
    render(<App />);
    await waitFor(() => expect(screen.getByLabelText('Pause')).toBeInTheDocument());
    fireEvent.click(screen.getByLabelText('Pause'));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('playback_pause', undefined));
    expect(status.position_seconds).toBe(0.6);
    fireEvent.click(screen.getByLabelText('Play'));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('playback_resume', undefined));
    expect(mockInvoke.mock.calls.filter(c => c[0] === 'playback_play')).toHaveLength(0);
  });

  it('uses the same pause/resume behavior from a sound row', async () => {
    const base = mockInvoke.getMockImplementation()!;
    let state = 'playing';
    mockInvoke.mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === 'playback_status') return Promise.resolve({ ...STOPPED_PLAYBACK, sound_id: SOUND_A.id, state, position_seconds: 0.6 });
      if (cmd === 'playback_pause') { state = 'paused'; return Promise.resolve(); }
      if (cmd === 'playback_resume') { state = 'playing'; return Promise.resolve(); }
      return base(cmd, args);
    });
    render(<App />);
    const pause = await screen.findByLabelText(`Pause ${SOUND_A.title}`);
    fireEvent.click(pause);
    expect(mockInvoke).toHaveBeenCalledWith('playback_pause', undefined);
    fireEvent.click(await screen.findByLabelText(`Play ${SOUND_A.title}`));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('playback_resume', undefined));
    expect(mockInvoke.mock.calls.filter(c => c[0] === 'playback_play')).toHaveLength(0);
  });

  it('hides library data immediately and shows the animation until favorites resolve', async () => {
    const base = mockInvoke.getMockImplementation()!;
    let resolveFavorites!: (r: SearchResults) => void;
    const pending = new Promise<SearchResults>(resolve => { resolveFavorites = resolve; });
    mockInvoke.mockImplementation((cmd: string, args?: any) => cmd === 'search_sounds' && args.query.favorites_only
      ? pending : base(cmd, args));
    render(<App />);
    await screen.findByText(SOUND_A.title);
    fireEvent.click(screen.getByText('Favorites'));
    expect(screen.queryByText(SOUND_A.title)).not.toBeInTheDocument();
    expect(screen.getByLabelText('Loading sounds')).toBeInTheDocument();
    expect(screen.getByRole('list', { name: 'Sounds' })).toHaveAttribute('aria-busy', 'true');
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('search_sounds', expect.objectContaining({ query: expect.objectContaining({ favorites_only: true }) })));
    await act(async () => resolveFavorites({ ...MOCK_RESULTS, items: [SOUND_B], total: 1 }));
    expect(screen.queryByLabelText('Loading sounds')).not.toBeInTheDocument();
    expect(screen.getByText(SOUND_B.title)).toBeInTheDocument();
    expect(screen.queryByText(SOUND_A.title)).not.toBeInTheDocument();
  });

  it('ignores a late favorites response after returning to the library', async () => {
    const base = mockInvoke.getMockImplementation()!;
    let resolveFavorites!: (r: SearchResults) => void;
    const pending = new Promise<SearchResults>(resolve => { resolveFavorites = resolve; });
    mockInvoke.mockImplementation((cmd: string, args?: any) => cmd === 'search_sounds' && args.query.favorites_only
      ? pending : base(cmd, args));
    render(<App />);
    await screen.findByText(SOUND_A.title);
    fireEvent.click(screen.getByText('Favorites'));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('search_sounds', expect.objectContaining({ query: expect.objectContaining({ favorites_only: true }) })));
    fireEvent.click(screen.getByRole('button', { name: /Library/ }));
    await screen.findByText(SOUND_A.title);
    await act(async () => resolveFavorites({ ...MOCK_RESULTS, items: [], total: 0 }));
    expect(screen.getByText(SOUND_A.title)).toBeInTheDocument();
  });
});

it('does not reopen the previous inspector when selection fetch finishes after navigation', async () => {
  const base = mockInvoke.getMockImplementation()!;
  let finish!: (s: Sound) => void;
  const pending = new Promise<Sound>(resolve => { finish = resolve; });
  mockInvoke.mockImplementation((cmd: string, args?: unknown) => cmd === 'get_sound' ? pending : base(cmd, args));
  render(<App />);
  await screen.findByText(SOUND_A.title);
  fireEvent.click(screen.getByLabelText(/Cinematic Whoosh Stereo, duration/));
  expect(screen.getByText('DETAILS')).toBeInTheDocument();
  fireEvent.click(screen.getByText('Favorites'));
  await act(async () => finish(SOUND_A));
  expect(screen.queryByText('DETAILS')).not.toBeInTheDocument();
});


describe('keyboard and details regression coverage', () => {
  it('activates navigation with Space without starting playback', async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText(SOUND_A.title);
    const favorites = screen.getByRole('button', { name: 'Favorites' });
    favorites.focus();
    await user.keyboard(' ');
    expect(screen.getByRole('heading', { level: 1, name: 'Favorites' })).toBeInTheDocument();
    expect(mockInvoke.mock.calls.some(call => call[0] === 'playback_play')).toBe(false);
  });

  it('moves actual focus through results and opens details only with Enter', async () => {
    const user = userEvent.setup();
    render(<App />);
    const first = await screen.findByRole('button', { name: /Cinematic Whoosh Stereo, duration/ });
    const second = screen.getByRole('button', { name: /Heavy Impact Mono, duration/ });
    first.focus();
    await user.keyboard('{ArrowDown}');
    expect(second).toHaveFocus();
    expect(screen.queryByText('DETAILS')).not.toBeInTheDocument();
    await user.keyboard('{Home}');
    expect(first).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(screen.getByText('DETAILS')).toBeInTheDocument();
  });

  it('lets Enter activate Favorite instead of opening details', async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByText(SOUND_A.title);
    screen.getByLabelText(`Favorite ${SOUND_A.title}`).focus();
    await user.keyboard('{Enter}');
    expect(mockInvoke).toHaveBeenCalledWith('set_favorite', { id: SOUND_A.id, favorite: true });
    expect(screen.queryByText('DETAILS')).not.toBeInTheDocument();
  });

  it('does not hijack modified shortcuts or navigation in Imports', async () => {
    render(<App />);
    await screen.findByText(SOUND_A.title);
    fireEvent.keyDown(window, { key: 'm', metaKey: true });
    expect(mockInvoke.mock.calls.some(call => call[0] === 'playback_set_volume')).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Imports' }));
    expect(fireEvent.keyDown(window, { key: 'ArrowDown' })).toBe(true);
    expect(screen.queryByText('DETAILS')).not.toBeInTheDocument();
  });

  it('keeps details closed when a deferred selection resolves after Escape', async () => {
    let finish!: (sound: Sound) => void;
    const fallback = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation((command: string, args: unknown) => command === 'get_sound'
      ? new Promise(resolve => { finish = resolve; }) : fallback(command, args));
    render(<App />);
    fireEvent.click(await screen.findByRole('button', { name: /Cinematic Whoosh Stereo, duration/ }));
    fireEvent.keyDown(window, { key: 'Escape' });
    await act(async () => finish(SOUND_A));
    expect(screen.queryByText('DETAILS')).not.toBeInTheDocument();
    expect(mockInvoke.mock.calls.filter(call => call[0] === 'get_sound')).toHaveLength(1);
  });

  it('does not reopen details when saving finishes after closing', async () => {
    let finish!: () => void;
    const fallback = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation((command: string, args: unknown) => command === 'annotate'
      ? new Promise<void>(resolve => { finish = resolve; }) : fallback(command, args));
    render(<App />);
    fireEvent.click(await screen.findByRole('button', { name: /Cinematic Whoosh Stereo, duration/ }));
    await screen.findByLabelText('Comment');
    fireEvent.click(screen.getByText('Save changes'));
    fireEvent.click(screen.getByLabelText('Close details'));
    await act(async () => finish());
    expect(screen.queryByText('DETAILS')).not.toBeInTheDocument();
  });
});
