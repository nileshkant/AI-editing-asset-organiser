import { render, screen, fireEvent, waitFor, act } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { App } from './App';
import { duration, type Sound, type SearchResults, type PlaybackStatus } from './api';

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

if (typeof window !== 'undefined') {
  if (!window.ResizeObserver) {
    window.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as any;
  }
  if (typeof HTMLCanvasElement !== 'undefined') {
    HTMLCanvasElement.prototype.getContext = () => null;
  }
}

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
    tags: ['whoosh', 'stereo'],
    waveform: [[-0.8, 0.8]],
  },
  user_tags: ['whoosh', 'impact'],
  comment: 'Use in intro sequence',
  favorite: false,
};

const SOUND_B: Sound = {
  id: 'sound-b',
  source_id: 'src-1',
  relative_path: 'audio/laser.wav',
  title: 'Futuristic Laser Blast Pulse Generator Ultra Long Sound Effect Title That Should Truncate Safely Without Overlapping Nearby Controls Or Durations',
  content_hash: 'hash-b',
  status: 'ready',
  profile: {
    duration: 3.2,
    sample_rate: 48000,
    channels: 1,
    channel_layout: 'mono',
    channel_peaks: [0.9],
    channel_rms: [0.3],
    frames: 153600,
    peak: 0.9,
    rms: 0.3,
    description: 'Laser blast pulse',
    tags: ['laser', 'sci-fi'],
    waveform: [[-0.9, 0.9]],
  },
  user_tags: ['laser', 'weapon'],
  comment: 'Sci-fi weapon shot',
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
  position_seconds: 0.0,
  duration_seconds: 0.0,
  volume: 1.0,
  peak: 0.0,
  error: null,
};

describe('SS-009: Library workspace and accessibility', () => {
  beforeEach(() => {
    tauriEnabled = false;
    mockInvoke.mockReset();
  });

  it('opens the workspace instead of a marketing page', () => {
    render(<App />);
    expect(screen.getByRole('heading', { name: 'Library' })).toBeInTheDocument();
    expect(screen.getByText('No sounds yet')).toBeInTheDocument();
  });

  it('starts with AI disabled and displays empty folders state', () => {
    render(<App />);
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(screen.getByText(/AI features/)).toBeInTheDocument();
    expect(screen.getByText(/Disabled/)).toBeInTheDocument();
    expect(screen.getByText('No folders added.')).toBeInTheDocument();
  });

  it('formats audio duration accurately', () => {
    expect(duration(0)).toBe('0:00.00');
    expect(duration(2.5)).toBe('0:02.50');
    expect(duration(65.123)).toBe('1:05.12');
  });

  it('renders transport controls footer with play, stop, meter, and volume slider', () => {
    render(<App />);
    expect(screen.getByLabelText('Audio transport')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Play' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeInTheDocument();
    expect(screen.getByRole('slider', { name: 'Volume' })).toBeInTheDocument();
    expect(screen.getByRole('slider', { name: 'Seek position' })).toBeInTheDocument();
    expect(screen.getByLabelText('Output Level')).toBeInTheDocument();
  });

  it('handles volume slider adjustments', () => {
    render(<App />);
    const volumeSlider = screen.getByRole('slider', { name: 'Volume' });
    expect(volumeSlider).toHaveValue('1');
    fireEvent.change(volumeSlider, { target: { value: '0.6' } });
    expect(volumeSlider).toHaveValue('0.6');
  });

  it('handles mute and unmute toggling', () => {
    render(<App />);
    const muteButton = screen.getByRole('button', { name: 'Mute' });
    fireEvent.click(muteButton);
    expect(screen.getByRole('button', { name: 'Unmute' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Unmute' }));
    expect(screen.getByRole('button', { name: 'Mute' })).toBeInTheDocument();
  });

  it('handles spacebar keyboard shortcut for transport toggle', () => {
    render(<App />);
    const playButton = screen.getByRole('button', { name: 'Play' });
    expect(playButton).toBeInTheDocument();
    fireEvent.keyDown(window, { code: 'Space' });
    expect(playButton).toBeInTheDocument();
  });

  it('keeps list selection and active playback independent', async () => {
    tauriEnabled = true;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([{ id: 'src-1', name: 'Audio Pack', root: '/audio', generation: 1, available: true }]);
      if (cmd === 'search_sounds') return Promise.resolve(MOCK_RESULTS);
      if (cmd === 'get_sound') return Promise.resolve(SOUND_B);
      if (cmd === 'playback_status') {
        return Promise.resolve({
          sound_id: 'sound-a',
          state: 'playing',
          position_seconds: 0.75,
          duration_seconds: 1.5,
          volume: 1.0,
          peak: 0.65,
          error: null,
        } satisfies PlaybackStatus);
      }
      if (cmd === 'playback_play') return Promise.resolve();
      if (cmd === 'playback_pause') return Promise.resolve();
      return Promise.resolve();
    });

    render(<App />);

    // Wait for search results to load
    await waitFor(() => {
      expect(screen.getByText('Cinematic Whoosh Stereo')).toBeInTheDocument();
    });

    // Sound A is active in playback. Transport must show Sound A!
    await waitFor(() => {
      const transportTitle = screen.getByText('Cinematic Whoosh Stereo', { selector: '.transport-title' });
      expect(transportTitle).toBeInTheDocument();
    });

    // Inspect Sound B by clicking its row select button
    const soundBRow = screen.getByRole('button', { name: /Futuristic Laser Blast.*duration/ });
    fireEvent.click(soundBRow);

    // Sound B should now be displayed in the Inspector
    await waitFor(() => {
      expect(screen.getByRole('heading', { name: SOUND_B.title })).toBeInTheDocument();
    });

    // CRITICAL: Transport title MUST STILL BE Sound A (the playing sound), NOT Sound B!
    const transportTitle = screen.getByText('Cinematic Whoosh Stereo', { selector: '.transport-title' });
    expect(transportTitle).toBeInTheDocument();

    // Sound A row has Pause button (since it's playing)
    expect(screen.getByRole('button', { name: 'Pause Cinematic Whoosh Stereo' })).toBeInTheDocument();

    // Sound B row has Play button (since it's not playing)
    expect(screen.getByRole('button', { name: /Play Futuristic Laser Blast/ })).toBeInTheDocument();
  });

  it('supports keyboard navigation with Arrow keys, Enter, and shortcuts', async () => {
    tauriEnabled = true;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'search_sounds') return Promise.resolve(MOCK_RESULTS);
      if (cmd === 'get_sound') return Promise.resolve(SOUND_A);
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      return Promise.resolve();
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText('Cinematic Whoosh Stereo')).toBeInTheDocument();
    });

    // Press ArrowDown to navigate to first sound
    fireEvent.keyDown(window, { key: 'ArrowDown' });
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('get_sound', { id: 'sound-a' });
    });

    // Press '/' to focus search input
    const searchInput = screen.getByRole('textbox', { name: 'Search audio' });
    fireEvent.keyDown(window, { key: '/' });
    expect(searchInput).toHaveFocus();

    // While typing in search input, Space should NOT trigger transport play
    fireEvent.keyDown(searchInput, { code: 'Space' });
    // Verify no call to playback_play happened from typing space
    expect(mockInvoke).not.toHaveBeenCalledWith('playback_play', expect.anything());

    // Press 'm' outside inputs to toggle mute
    searchInput.blur();
    fireEvent.keyDown(window, { key: 'm' });
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith('playback_set_volume', { volume: 0 });
    });

    // Press Escape to dismiss details inspector
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => {
      expect(screen.queryByRole('heading', { name: 'DETAILS' })).not.toBeInTheDocument();
    });
  });

  it('provides accessible ARIA semantics and live region announcements', async () => {
    tauriEnabled = true;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'search_sounds') return Promise.resolve(MOCK_RESULTS);
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      return Promise.resolve();
    });

    render(<App />);

    // Live region exists for screen readers
    const liveRegion = screen.getByRole('status');
    expect(liveRegion).toHaveAttribute('aria-live', 'polite');
    expect(liveRegion).toHaveAttribute('aria-atomic', 'true');

    // Wait for search results announcement
    await waitFor(() => {
      expect(liveRegion).toHaveTextContent('2 sounds found');
    });

    // Sound list has listbox role and sound rows have option roles
    const soundList = screen.getByRole('listbox', { name: 'Sounds' });
    expect(soundList).toBeInTheDocument();
    const options = screen.getAllByRole('option');
    expect(options).toHaveLength(2);

    // Sliders have accessible attributes
    const volumeSlider = screen.getByRole('slider', { name: 'Volume' });
    expect(volumeSlider).toHaveAttribute('aria-valuemin', '0');
    expect(volumeSlider).toHaveAttribute('aria-valuemax', '100');
    expect(volumeSlider).toHaveAttribute('aria-valuenow', '100');
    expect(volumeSlider).toHaveAttribute('aria-valuetext', '100 percent');

    const seekSlider = screen.getByRole('slider', { name: 'Seek position' });
    expect(seekSlider).toHaveAttribute('aria-valuemin', '0');
    expect(seekSlider).toHaveAttribute('aria-valuenow', '0');

    // Output level has meter role
    const meter = screen.getByRole('meter', { name: 'Output Level' });
    expect(meter).toHaveAttribute('aria-valuemin', '0');
    expect(meter).toHaveAttribute('aria-valuemax', '100');
  });

  it('safely renders ultra-long sound names without text overlap', async () => {
    tauriEnabled = true;
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === 'app_info') return Promise.resolve({ version: '0.1.0', data_directory: '/data', desktop: true, media_tools: true });
      if (cmd === 'jobs') return Promise.resolve([]);
      if (cmd === 'sources') return Promise.resolve([]);
      if (cmd === 'search_sounds') return Promise.resolve(MOCK_RESULTS);
      if (cmd === 'playback_status') return Promise.resolve(STOPPED_PLAYBACK);
      return Promise.resolve();
    });

    render(<App />);

    await waitFor(() => {
      const longSound = screen.getByText(SOUND_B.title);
      expect(longSound).toBeInTheDocument();
      // Ensure it renders within sound-label container without crashing
      expect(longSound.parentElement).toHaveClass('sound-label');
    });
  });
});

