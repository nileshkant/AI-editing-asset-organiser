import { act, cleanup, fireEvent, render, renderHook, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ImageSources } from './components/views/ImageSources';
import { jobLabel } from './components/views/ImageImports';
import { RelinkModal } from './components/modals/RelinkModal';
import { useImageSources } from './hooks/useImageSources';
import type { ImageImportJob, ImageMoveCandidate, ImageRelinkPreview, ImageSourceStatus, Source } from './types';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));

const status = (over: Partial<ImageSourceStatus> = {}): ImageSourceStatus => ({
  source_id: 's1', name: 'Shoot', root: '/Volumes/Drive/Shoot', scope: 'folder', source_generation: '0', root_state: 'online',
  watch: false, mode: 'images', checked_at: 0, images: 4, missing: 1, inaccessible: 0, failed: 2, pending: 0,
  move_candidates: 0, active_job: null, watch_state: null, ...over,
});
const job: ImageImportJob = { id: 'j', source_id: 's1', source_generation: '0', sequence: '1', mode: 'images', state: 'queued', phase: 'discovery', discovery_complete: false, history_truncated: false, total: 0, completed: 0, ready: 0, reused: 0, failed: 0, unsupported: 0, current: '', error_code: null, scan: 'discover', verify: false, origin: 'refresh' };
let sources: ImageSourceStatus[];
let moves: ImageMoveCandidate[];
beforeEach(() => {
  invoke.mockReset();
  sources = [status()];
  moves = [];
  invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === 'image_source_statuses') return Promise.resolve(sources);
    if (command === 'set_image_source_watch') { sources = [status({ watch: !!args?.enabled, watch_state: { state: 'watching', error: null } })]; return Promise.resolve(sources[0]); }
    if (command === 'refresh_image_source') return Promise.resolve(job);
    if (command === 'image_move_candidates') return Promise.resolve(moves);
    if (command === 'reconcile_image_move') { moves = []; return Promise.resolve({}); }
    return Promise.resolve(sources[0]);
  });
});
afterEach(cleanup);
function Harness({ onRelink = vi.fn(), onError = vi.fn() }) {
  const c = useImageSources(true, [], onError);
  return <ImageSources controller={c} onRelink={onRelink} />;
}

describe('SS-047 image sources', () => {
  it('distinguishes root states and sends each refresh kind with camelCase IPC', async () => {
    render(<Harness />);
    const card = await screen.findByRole('article', { name: 'Image source Shoot' });
    expect(card).toHaveTextContent('Available');
    expect(card).toHaveTextContent('4 images · 1 missing · 0 access denied · 2 could not be read');
    expect(card).toHaveTextContent('Off. Watching uses system notifications for this folder only.');
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('refresh_image_source', { id: 's1', scan: 'discover', verify: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Check files' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('refresh_image_source', { id: 's1', scan: 'listed', verify: false }));
    fireEvent.click(screen.getByRole('button', { name: 'Verify file bytes' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('refresh_image_source', { id: 's1', scan: 'listed', verify: true }));
    fireEvent.click(screen.getByRole('button', { name: 'Check availability' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('check_image_source', { id: 's1' }));
  });
  it('makes watching an explicit opt-in and explains paused or failed watchers', async () => {
    render(<Harness />);
    const toggle = await screen.findByRole('checkbox', { name: 'Watch for changes' });
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('set_image_source_watch', { id: 's1', enabled: true }));
    await waitFor(() => expect(screen.getByRole('checkbox', { name: 'Watch for changes' })).toBeChecked());
    expect(screen.getByText(/Watching\. Changes are checked by size and date/)).toBeInTheDocument();
    cleanup();
    sources = [status({ watch: true, root_state: 'offline', watch_state: { state: 'paused', error: 'Source is offline' } }), status({ source_id: 's2', name: 'Picks', scope: 'files', root_state: 'inaccessible' }), status({ source_id: 's3', name: 'Swap', watch: true, root_state: 'changed', watch_state: { state: 'failed', error: 'inotify watch limit reached' } })];
    render(<Harness />);
    const offline = await screen.findByRole('article', { name: 'Image source Shoot' });
    expect(offline).toHaveTextContent('Offline · reconnect the drive');
    expect(offline).toHaveTextContent('Paused · Source is offline');
    for (const name of ['Refresh', 'Check files', 'Verify file bytes']) expect(within(offline).getByRole('button', { name })).toBeDisabled();
    expect(within(offline).getByRole('button', { name: 'Check availability' })).toBeEnabled();
    const files = screen.getByRole('article', { name: 'Image source Picks' });
    expect(files).toHaveTextContent('Access denied');
    expect(files).toHaveTextContent('Watching covers only the selected files, never their siblings.');
    expect(files.querySelector('button[title="Find new, changed and removed files"]')).toBeNull();
    const swapped = screen.getByRole('article', { name: 'Image source Swap' });
    expect(swapped).toHaveTextContent('relink to verify it');
    expect(swapped).toHaveTextContent('Unavailable · inotify watch limit reached. Use Refresh instead.');
  });
  it('blocks overlapping work while a job runs and reattaches a move with expected revisions', async () => {
    const onRelink = vi.fn();
    moves = [{ missing_id: 'm', missing_path: 'old/a.png', missing_revision: 4, present_id: 'p', present_path: 'new/a.png', present_revision: 1, same_file: false }];
    sources = [status({ move_candidates: 1, active_job: 'j' })];
    render(<Harness onRelink={onRelink} />);
    await screen.findByText('A check is running; see its progress below.');
    expect(screen.getByRole('button', { name: 'Refresh' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Relink…' })).toBeDisabled();
    fireEvent.click(screen.getByText('Possible moved files (1)'));
    const reattach = await screen.findByRole('button', { name: 'Reattach old/a.png to new/a.png' });
    expect(reattach).toBeDisabled();
    cleanup();
    sources = [status({ move_candidates: 1 })];
    render(<Harness onRelink={onRelink} />);
    fireEvent.click(await screen.findByText('Possible moved files (1)'));
    fireEvent.click(await screen.findByRole('button', { name: 'Reattach old/a.png to new/a.png' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('reconcile_image_move', { missingId: 'm', presentId: 'p', missingRevision: 4, presentRevision: 1 }));
    await screen.findByText('No possible moves remain.');
    fireEvent.click(screen.getByRole('button', { name: 'Relink…' }));
    expect(onRelink).toHaveBeenCalledWith('s1');
  });
  it('reports action failures through the shared error banner and still re-reads status', async () => {
    const onError = vi.fn();
    invoke.mockImplementation((command: string) => command === 'image_source_statuses' ? Promise.resolve(sources) : command === 'refresh_image_source' ? Promise.reject('Another selection for this source is already importing') : Promise.resolve(null));
    const { result } = renderHook(() => useImageSources(true, [], onError));
    await waitFor(() => expect(result.current.sources).toHaveLength(1));
    const calls = () => invoke.mock.calls.filter(([c]) => c === 'image_source_statuses').length;
    const before = calls();
    await act(async () => { await result.current.act(() => invoke('refresh_image_source')); });
    expect(onError).toHaveBeenCalledWith('Another selection for this source is already importing');
    expect(calls()).toBeGreaterThan(before);
  });
  it('labels jobs by what they check', () => {
    expect(jobLabel(job)).toBe('Refresh · images');
    expect(jobLabel({ ...job, scan: 'listed', verify: true, mode: 'mixed' })).toBe('Byte verification · images and audio');
    expect(jobLabel({ ...job, scan: 'listed' })).toBe('File check · images');
    expect(jobLabel({ ...job, origin: 'watch' })).toBe('Watched changes · images');
    expect(jobLabel({ ...job, origin: 'relink', scan: 'listed', verify: true })).toBe('Relink verification · images');
    expect(jobLabel({ ...job, origin: 'import', verify: true })).toBe('Images');
  });
});

describe('SS-047 image relink', () => {
  const source: Source = { id: 's1', name: 'Shoot', root: '/Volumes/Old/Shoot', generation: 0, available: false, scope: 'folder', files: [] };
  const preview = (over: Partial<ImageRelinkPreview> = {}): ImageRelinkPreview => ({ id: 'plan', source_id: 's1', source_generation: '0', root: '/Volumes/New/Shoot', members: 5, present: 4, absent: 1, sampled: 4, matched: 3, changed: 1, unreadable: 0, complete: true, acknowledgement_required: true, expires_at: 0, ...over });
  it('uses only the native picker, discloses counts and needs consent for changes', async () => {
    const user = userEvent.setup();
    const relinked = vi.fn();
    invoke.mockImplementation((command: string) => Promise.resolve(command === 'choose_image_relink' ? preview() : command === 'confirm_image_relink' ? job : null));
    render(<RelinkModal source={source} image onClose={vi.fn()} onRelinked={relinked} onError={vi.fn()} />);
    expect(screen.queryByLabelText('Folder path')).toBeNull();
    const choose = screen.getByRole('button', { name: 'Choose folder' });
    expect(choose).toHaveFocus();
    expect(screen.getByRole('button', { name: 'Relink and verify' })).toBeDisabled();
    await user.click(choose);
    expect(invoke).toHaveBeenCalledWith('choose_image_relink', { id: 's1' });
    expect(await screen.findByText('4 of 5 cataloged files found · 1 not found')).toBeInTheDocument();
    expect(screen.getByText('Sample check: 3 of 4 identical · 1 changed · 0 unreadable')).toBeInTheDocument();
    const confirm = screen.getByRole('button', { name: 'Relink and verify' });
    expect(confirm).toBeDisabled();
    await user.click(screen.getByRole('checkbox'));
    await user.click(confirm);
    expect(invoke).toHaveBeenCalledWith('confirm_image_relink', { previewId: 'plan', acknowledged: true });
    await waitFor(() => expect(relinked).toHaveBeenCalled());
  });
  it('discards an unconfirmed preview on Escape and keeps focus trapped', async () => {
    const user = userEvent.setup();
    const close = vi.fn();
    invoke.mockImplementation((command: string) => Promise.resolve(command === 'choose_image_relink' ? preview({ acknowledgement_required: false, absent: 0, changed: 0 }) : null));
    const view = render(<RelinkModal source={source} image onClose={close} onRelinked={vi.fn()} onError={vi.fn()} />);
    await user.click(screen.getByRole('button', { name: 'Choose folder' }));
    const confirm = await screen.findByRole('button', { name: 'Relink and verify' });
    await waitFor(() => expect(confirm).toBeEnabled());
    confirm.focus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'Cancel' })).toHaveFocus();
    await user.keyboard('{Escape}');
    expect(close).toHaveBeenCalledTimes(1);
    view.unmount();
    expect(invoke).toHaveBeenCalledWith('cancel_image_relink', { previewId: 'plan' });
  });
  it('treats a dismissed picker as no change and shows failures in the dialog', async () => {
    const user = userEvent.setup();
    const onError = vi.fn();
    invoke.mockImplementationOnce(() => Promise.resolve(null)).mockImplementationOnce(() => Promise.reject("None of this source's cataloged files are in that folder"));
    render(<RelinkModal source={source} image onClose={vi.fn()} onRelinked={vi.fn()} onError={onError} />);
    await user.click(screen.getByRole('button', { name: 'Choose folder' }));
    expect(screen.queryByRole('alert')).toBeNull();
    await user.click(screen.getByRole('button', { name: 'Choose folder' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('None of this source');
    expect(onError).toHaveBeenCalled();
  });
});
