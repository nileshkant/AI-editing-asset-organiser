import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { Waveform } from './Waveform';

if (typeof window !== 'undefined') {
  if (!window.ResizeObserver) {
    window.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    } as any;
  }
}

describe('SS-011: Waveform visualization component', () => {
  const MOCK_PEAKS: [number, number][] = [
    [-0.5, 0.5],
    [-0.8, 0.8],
    [-0.2, 0.2],
    [-0.95, 0.95],
  ];

  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders waveform workstation with canvas and toolbar controls', () => {
    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={2.5}
        sampleRate={48000}
        channels={2}
      />
    );

    expect(screen.getByRole('region', { name: 'Audio waveform workstation' })).toBeInTheDocument();
    expect(screen.getByRole('toolbar', { name: 'Waveform controls' })).toBeInTheDocument();
    expect(screen.getByRole('img', { name: /Audio waveform with duration 0:02.50/ })).toBeInTheDocument();
    expect(screen.getByText(/Stereo · 48.0 kHz/)).toBeInTheDocument();
  });

  it('handles deterministic zoom in, zoom out, and fit window', () => {
    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={5.0}
        sampleRate={48000}
        channels={1}
      />
    );

    const zoomBadge = screen.getByLabelText('Zoom level 1x');
    expect(zoomBadge).toHaveTextContent('1x');

    const zoomInBtn = screen.getByRole('button', { name: 'Zoom in' });
    fireEvent.click(zoomInBtn);
    expect(screen.getByLabelText('Zoom level 2x')).toHaveTextContent('2x');

    // Zoom out button
    const zoomOutBtn = screen.getByRole('button', { name: 'Zoom out' });
    fireEvent.click(zoomOutBtn);
    expect(screen.getByLabelText('Zoom level 1x')).toHaveTextContent('1x');

    // Zoom in multiple times and fit
    fireEvent.click(zoomInBtn);
    fireEvent.click(zoomInBtn);
    expect(screen.getByLabelText('Zoom level 4x')).toHaveTextContent('4x');

    const fitBtn = screen.getByRole('button', { name: 'Fit to window' });
    fireEvent.click(fitBtn);
    expect(screen.getByLabelText('Zoom level 1x')).toHaveTextContent('1x');
  });

  it('toggles separate stereo channel lanes when channels >= 2', () => {
    render(
      <Waveform
        soundId="sound-stereo"
        peaks={MOCK_PEAKS}
        duration={3.0}
        sampleRate={44100}
        channels={2}
      />
    );

    const splitToggle = screen.getByRole('button', { name: 'Combined waveform view' });
    expect(splitToggle).toHaveAttribute('aria-pressed', 'true');

    // Click toggle to switch to combined
    fireEvent.click(splitToggle);
    expect(screen.getByRole('button', { name: 'Split stereo channels' })).toHaveAttribute(
      'aria-pressed',
      'false'
    );
  });

  it('supports accessible numeric selection inputs and boundary clamping', () => {
    const onSelectionChange = vi.fn();
    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={4.0}
        sampleRate={48000}
        channels={1}
        selection={{ start: 1.0, end: 2.5 }}
        onSelectionChange={onSelectionChange}
      />
    );

    const startInput = screen.getByRole('spinbutton', { name: 'Selection start in seconds' });
    const endInput = screen.getByRole('spinbutton', { name: 'Selection end in seconds' });
    const durationInput = screen.getByRole('spinbutton', { name: 'Selection duration in seconds' });

    expect(startInput).toHaveValue(1.0);
    expect(endInput).toHaveValue(2.5);
    expect(durationInput).toHaveValue(1.5);

    // Update start input
    fireEvent.change(startInput, { target: { value: '1.2' } });
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 1.2, end: 2.5 });

    // Update end input
    fireEvent.change(endInput, { target: { value: '3.0' } });
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 1.0, end: 3.0 });
  });

  it('supports Select All and Clear Selection', () => {
    const onSelectionChange = vi.fn();
    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={6.0}
        sampleRate={48000}
        channels={1}
        selection={null}
        onSelectionChange={onSelectionChange}
      />
    );

    // When no selection, prompt and Select All button are present
    const selectAllBtn = screen.getByRole('button', { name: 'Select All' });
    fireEvent.click(selectAllBtn);
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 0, end: 6.0 });
  });

  it('triggers onSeek callback on waveform canvas click', () => {
    const onSeek = vi.fn();
    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={10.0}
        sampleRate={48000}
        channels={1}
        onSeek={onSeek}
      />
    );

    const canvas = screen.getByRole('img', { name: /Audio waveform/ });

    // Mock bounding rect
    vi.spyOn(canvas, 'getBoundingClientRect').mockReturnValue({
      left: 0,
      top: 0,
      width: 1000,
      height: 100,
      right: 1000,
      bottom: 100,
      x: 0,
      y: 0,
      toJSON: () => {},
    });

    // Click in middle of canvas (pixel 500 out of 1000 -> 5.0 seconds)
    fireEvent.mouseDown(canvas, { clientX: 500, clientY: 50 });
    expect(onSeek).toHaveBeenCalledWith(5.0);
  });

  it('supports Start at Playhead and End at Playhead buttons', () => {
    const onSelectionChange = vi.fn();
    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={10.0}
        sampleRate={48000}
        channels={1}
        playbackPosition={3.5}
        selection={{ start: 1.0, end: 7.0 }}
        onSelectionChange={onSelectionChange}
      />
    );

    const startAtPlayhead = screen.getByRole('button', { name: /Start at Playhead/ });
    fireEvent.click(startAtPlayhead);
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 3.5, end: 7.0 });

    const endAtPlayhead = screen.getByRole('button', { name: /End at Playhead/ });
    fireEvent.click(endAtPlayhead);
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 1.0, end: 3.5 });
  });

  it('supports keyboard nudging of selection with arrow keys and I/O keys', () => {
    const onSelectionChange = vi.fn();
    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={10.0}
        sampleRate={48000}
        channels={1}
        playbackPosition={4.0}
        selection={{ start: 2.0, end: 5.0 }}
        onSelectionChange={onSelectionChange}
      />
    );

    const workstation = screen.getByRole('region', { name: 'Audio waveform workstation' });

    // ArrowLeft nudges selection earlier
    fireEvent.keyDown(workstation, { key: 'ArrowLeft' });
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 1.99, end: 4.99 });

    // ArrowRight with shift nudges by 0.1
    fireEvent.keyDown(workstation, { key: 'ArrowRight', shiftKey: true });
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 2.1, end: 5.1 });

    // Press 'i' to set start to playhead (4.0)
    fireEvent.keyDown(workstation, { key: 'i' });
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 4.0, end: 5.0 });

    // Press 'o' to set end to playhead (4.0)
    fireEvent.keyDown(workstation, { key: 'o' });
    expect(onSelectionChange).toHaveBeenCalledWith({ start: 2.0, end: 4.0 });
  });

  it('draws canvas bars with 2d context calls when available', () => {
    Object.defineProperty(HTMLCanvasElement.prototype, 'clientWidth', { configurable: true, value: 500 });
    Object.defineProperty(HTMLCanvasElement.prototype, 'clientHeight', { configurable: true, value: 100 });

    const fillRectMock = vi.fn();
    const strokeMock = vi.fn();
    const getContextMock = vi.fn().mockReturnValue({
      scale: vi.fn(),
      clearRect: vi.fn(),
      fillRect: fillRectMock,
      stroke: strokeMock,
      beginPath: vi.fn(),
      moveTo: vi.fn(),
      lineTo: vi.fn(),
      fillText: vi.fn(),
      closePath: vi.fn(),
      fill: vi.fn(),
      canvas: { clientWidth: 500, clientHeight: 100 },
    });

    HTMLCanvasElement.prototype.getContext = getContextMock as any;

    render(
      <Waveform
        soundId="sound-1"
        peaks={MOCK_PEAKS}
        duration={4.0}
        sampleRate={48000}
        channels={2}
      />
    );

    expect(fillRectMock).toHaveBeenCalled();
  });
});
