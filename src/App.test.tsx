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
});
