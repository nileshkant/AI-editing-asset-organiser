import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { RelinkModal } from './components/modals/RelinkModal';
import type { Source } from './types';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
const source: Source = { id: 'source', name: 'Recordings', root: '/missing', generation: 0, available: false, scope: 'folder', files: [] };

describe('relink keyboard and failure recovery', () => {
  it('traps focus, dismisses with Escape, and restores the opener', async () => {
    const user = userEvent.setup();
    const opener = document.createElement('button');
    document.body.append(opener); opener.focus();
    const close = vi.fn();
    const view = render(<RelinkModal source={source} onClose={close} onRelinked={vi.fn()} onError={vi.fn()} />);
    expect(screen.getByLabelText('Folder path')).toHaveFocus();
    await user.keyboard('{Shift>}{Tab}{/Shift}');
    expect(screen.getByText('Verify & relink')).toHaveFocus();
    await user.tab();
    expect(screen.getByLabelText('Folder path')).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(close).toHaveBeenCalledTimes(1);
    view.unmount();
    expect(opener).toHaveFocus(); opener.remove();
  });

  it('submits once with Enter and shows failures inside the modal', async () => {
    const user = userEvent.setup();
    let reject!: (error: Error) => void;
    invoke.mockReset().mockImplementation(() => new Promise((_, fail) => { reject = fail; }));
    const error = vi.fn(); const close = vi.fn();
    render(<RelinkModal source={source} onClose={close} onRelinked={vi.fn()} onError={error} />);
    await user.keyboard('{Enter}');
    fireEvent.submit(screen.getByText('Verify & relink').closest('form')!);
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(screen.getByText('Verify & relink')).toBeDisabled();
    await user.keyboard('{Escape}');
    expect(close).not.toHaveBeenCalled();
    await act(async () => reject(new Error('Folder does not match this source')));
    expect(screen.getByRole('alert')).toHaveTextContent('Folder does not match');
    expect(screen.getByText('Verify & relink')).toBeEnabled();
    expect(error).toHaveBeenCalledWith(expect.stringContaining('Folder does not match'));
  });
});
