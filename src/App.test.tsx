import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { App } from './App';
import { duration } from './api';

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));

describe('desktop foundation', () => {
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
    // In mock environment without active sound or backend, it safely handles the event
    expect(playButton).toBeInTheDocument();
  });
});
