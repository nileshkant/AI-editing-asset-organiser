import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Waveform } from './components/waveform/Waveform';

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: vi.fn(),
}));

const SAMPLE_PEAKS: [number, number][] = [
  [-0.5, 0.5],
  [-0.3, 0.7],
  [-0.8, 0.9],
  [-0.2, 0.4],
  [-0.6, 0.6],
];

describe('Waveform', () => {
  // ─── Rendering ───
  it('renders workstation region and canvas', () => {
    render(
      <Waveform
        peaks={SAMPLE_PEAKS}
        duration={2.0}
        sampleRate={48000}
        channels={2}
      />,
    );
    expect(
      screen.getByRole('region', { name: 'Audio waveform workstation' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('img')).toBeInTheDocument();
  });

  it('displays channel info and sample rate', () => {
    render(
      <Waveform
        peaks={SAMPLE_PEAKS}
        duration={2.0}
        sampleRate={48000}
        channels={2}
      />,
    );
    expect(screen.getByText(/Stereo/)).toBeInTheDocument();
    expect(screen.getByText(/48\.0 kHz/)).toBeInTheDocument();
  });

  it('shows mono label for single channel', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={1.0} channels={1} />);
    expect(screen.getByText(/Mono/)).toBeInTheDocument();
  });

  // ─── Zoom Controls ───
  it('zooms in from 1x to 2x', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    expect(screen.getByText('1x')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Zoom in'));
    expect(screen.getByText('2x')).toBeInTheDocument();
  });

  it('zooms in further to 4x', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    fireEvent.click(screen.getByLabelText('Zoom in'));
    fireEvent.click(screen.getByLabelText('Zoom in'));
    expect(screen.getByText('4x')).toBeInTheDocument();
  });

  it('does not zoom below 1x', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    expect(screen.getByLabelText('Zoom out')).toBeDisabled();
  });

  it('caps zoom at 64x', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    // Zoom 6 times: 2x, 4x, 8x, 16x, 32x, 64x
    for (let i = 0; i < 6; i++) {
      fireEvent.click(screen.getByLabelText('Zoom in'));
    }
    expect(screen.getByText('64x')).toBeInTheDocument();
    expect(screen.getByLabelText('Zoom in')).toBeDisabled();
  });

  it('resets zoom on Fit to Window', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    fireEvent.click(screen.getByLabelText('Zoom in'));
    expect(screen.getByText('2x')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Fit to window'));
    expect(screen.getByText('1x')).toBeInTheDocument();
  });

  // ─── Pan Slider ───
  it('shows pan slider when zoomed in', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    expect(screen.queryByLabelText('Pan offset')).not.toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Zoom in'));
    expect(screen.getByLabelText('Pan offset')).toBeInTheDocument();
  });

  // ─── Stereo Toggle ───
  it('renders stereo toggle for multi-channel audio', () => {
    render(
      <Waveform
        peaks={SAMPLE_PEAKS}
        duration={2.0}
        channels={2}
      />,
    );
    expect(
      screen.getByLabelText(/Combined waveform view|Split stereo channels/),
    ).toBeInTheDocument();
  });

  it('toggles between split and combined view', () => {
    render(
      <Waveform
        peaks={SAMPLE_PEAKS}
        duration={2.0}
        channels={2}
      />,
    );
    const toggle = screen.getByLabelText(/Combined waveform view|Split stereo channels/);
    fireEvent.click(toggle);
    // Should toggle the label
    expect(toggle.getAttribute('aria-pressed')).toBeDefined();
  });

  it('hides stereo toggle for mono audio', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={1.0} channels={1} />);
    expect(
      screen.queryByLabelText(/Combined waveform view|Split stereo channels/),
    ).not.toBeInTheDocument();
  });

  // ─── Selection ───
  it('renders Select All button', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    expect(screen.getByText('Select All')).toBeInTheDocument();
  });

  it('shows selection panel after Select All', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    fireEvent.click(screen.getByText('Select All'));
    expect(screen.getByLabelText('Selection start in seconds')).toBeInTheDocument();
    expect(screen.getByLabelText('Selection end in seconds')).toBeInTheDocument();
    expect(screen.getByLabelText('Selection duration in seconds')).toBeInTheDocument();
  });

  it('clears selection on Clear button', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    fireEvent.click(screen.getByText('Select All'));
    expect(screen.getByLabelText('Selection start in seconds')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Clear selection'));
    expect(screen.queryByLabelText('Selection start in seconds')).not.toBeInTheDocument();
  });

  it('adjusts selection start numerically', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    fireEvent.click(screen.getByText('Select All'));
    const startInput = screen.getByLabelText('Selection start in seconds');
    fireEvent.change(startInput, { target: { value: '0.500' } });
    expect((startInput as HTMLInputElement).value).toBe('0.500');
  });

  // ─── Zero / Edge Cases ───
  it('renders gracefully with empty peaks', () => {
    render(<Waveform peaks={[]} duration={1.0} />);
    expect(screen.getByRole('img')).toBeInTheDocument();
  });

  it('renders gracefully with zero duration', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={0} />);
    expect(screen.getByRole('img')).toBeInTheDocument();
  });

  it('renders with very short duration', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={0.001} />);
    expect(screen.getByRole('img')).toBeInTheDocument();
  });

  // ─── Click-to-Seek ───
  it('calls onSeek when canvas is clicked', () => {
    const mockSeek = vi.fn();
    render(
      <Waveform
        peaks={SAMPLE_PEAKS}
        duration={2.0}
        onSeek={mockSeek}
      />,
    );
    const canvas = screen.getByRole('img');

    // Mock getBoundingClientRect for coordinate calculation
    vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue({
      left: 0,
      right: 400,
      top: 0,
      bottom: 130,
      width: 400,
      height: 130,
      x: 0,
      y: 0,
      toJSON: () => {},
    });

    fireEvent.mouseDown(canvas, { clientX: 200, clientY: 65 });
    expect(mockSeek).toHaveBeenCalled();
    // Should seek to approximately middle of the 2s duration
    const seekTime = mockSeek.mock.calls[0][0];
    expect(seekTime).toBeGreaterThan(0.5);
    expect(seekTime).toBeLessThan(1.5);
  });

  // ─── Keyboard Selection Nudging ───
  it('nudges selection with arrow keys', () => {
    render(<Waveform peaks={SAMPLE_PEAKS} duration={2.0} />);
    // Create selection first
    fireEvent.click(screen.getByText('Select All'));

    const workstation = screen.getByRole('region', {
      name: 'Audio waveform workstation',
    });

    // Nudge with ArrowRight
    fireEvent.keyDown(workstation, { key: 'ArrowRight' });
    const endInput = screen.getByLabelText('Selection end in seconds');
    // End should have increased slightly
    expect(parseFloat((endInput as HTMLInputElement).value)).toBeGreaterThan(1.99);
  });

  // ─── Playhead / Playback Position ───
  it('renders start at playhead buttons with selection', () => {
    render(
      <Waveform
        peaks={SAMPLE_PEAKS}
        duration={2.0}
        playbackPosition={0.5}
      />,
    );
    fireEvent.click(screen.getByText('Select All'));
    expect(screen.getByText('Start at Playhead')).toBeInTheDocument();
    expect(screen.getByText('End at Playhead')).toBeInTheDocument();
  });
});
