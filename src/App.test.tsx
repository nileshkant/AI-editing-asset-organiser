import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { App } from './App';
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));
describe('desktop foundation', () => {
  it('opens the workspace instead of a marketing page', () => { render(<App/>); expect(screen.getByRole('heading', {name:'Library'})).toBeInTheDocument(); expect(screen.getByText('No sounds yet')).toBeInTheDocument(); });
  it('starts with AI disabled', () => { render(<App/>); fireEvent.click(screen.getByRole('button',{name:'Settings'})); expect(screen.getByText('Disabled')).toBeInTheDocument(); });
});
