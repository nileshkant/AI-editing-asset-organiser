import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ImageImports } from './components/views/ImageImports';
import { mergeImageJob, useImageImports } from './hooks/useImageImports';
import { useDragDrop } from './hooks/useDragDrop';
import type { ImageImportJob, MediaImportPreview } from './types';
const invoke = vi.fn(), unlisten = vi.fn();
const listeners = new Map<string, (event: { payload: unknown }) => void>();
let rawDrop: ((event: { payload: { type: string; paths: string[] } }) => void) | undefined;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn((name: string, fn: (event: { payload: unknown }) => void) => { listeners.set(name, fn); return Promise.resolve(unlisten); }) }));
vi.mock('@tauri-apps/api/webviewWindow', () => ({ getCurrentWebviewWindow: () => ({ onDragDropEvent: (fn: typeof rawDrop) => { rawDrop = fn; return Promise.resolve(unlisten); } }) }));
const job: ImageImportJob = { id: 'j1', source_id: 's1', source_generation: '0', sequence: '3', mode: 'images', state: 'running', phase: 'analysis', discovery_complete: true, history_truncated: false, total: 2, completed: 0, ready: 0, reused: 0, failed: 0, unsupported: 0, current: '', error_code: null };
const preview: MediaImportPreview = { id: 'preview', request_id: 'request', mode: 'mixed', scopes: [{ root: '/素材 long folder', recursive: false, files: ['chosen.png'] }], images: 1, audio: 0, unsupported: 1, inaccessible: 0, complete: true, issues: [], reference_in_place: true, expires_at: Math.floor(Date.now() / 1000) + 300 };
let backendJobs: ImageImportJob[];
beforeEach(() => {
  listeners.clear(); invoke.mockReset(); unlisten.mockReset(); backendJobs = [];
  invoke.mockImplementation((command: string) => { if (command === 'confirm_media_import') backendJobs = [job]; return Promise.resolve(command === 'image_import_capabilities' ? { enabled: true } : command === 'image_import_jobs' ? backendJobs : command === 'choose_media_import' ? ['/素材 long folder/chosen.png'] : command === 'preview_media_import' ? preview : command === 'confirm_media_import' ? [job] : command === 'image_import_items' ? { items: [], next_after: null } : command === 'imported_images' ? { items: [], next_after: null } : null); });
});
afterEach(cleanup);
function Harness() { const c = useImageImports(vi.fn(), vi.fn()); return <ImageImports controller={c} />; }
describe('gated native media imports', () => {
  it('previews exact scope, makes no import before confirmation, restores focus and sends camelCase IPC', async () => {
    render(<Harness />); const opener = await screen.findByRole('button', { name: 'Choose media files' }); opener.focus(); fireEvent.click(opener);
    const dialog = await screen.findByRole('dialog'); expect(dialog).toHaveTextContent('Selected files only'); expect(dialog).toHaveTextContent('1 images · 0 audio · 1 unsupported'); expect(dialog).toHaveTextContent('Originals are never modified');
    expect(invoke.mock.calls.some(([cmd]) => cmd === 'confirm_media_import')).toBe(false);
    expect(invoke).toHaveBeenCalledWith('choose_media_import', { folders: false });
    expect(invoke).toHaveBeenCalledWith('preview_media_import', { requestId: expect.any(String), paths: ['/素材 long folder/chosen.png'], mode: 'mixed' });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm import' })); await waitFor(() => expect(invoke).toHaveBeenCalledWith('confirm_media_import', { previewId: 'preview' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument()); expect(opener).toHaveFocus(); expect(screen.getByText(/0 \/ 2 files/)).toBeInTheDocument();
  });
  it('keeps keyboard focus in preview and Escape discards its opaque token', async () => {
    render(<Harness />); const opener = await screen.findByRole('button', { name: 'Choose media folders' }); opener.focus(); fireEvent.click(opener);
    await screen.findByRole('dialog'); const cancel = screen.getByRole('button', { name: 'Cancel' }), confirm = screen.getByRole('button', { name: 'Confirm import' });
    expect(cancel).toHaveFocus(); confirm.focus(); fireEvent.keyDown(confirm, { key: 'Tab' }); expect(cancel).toHaveFocus(); fireEvent.keyDown(cancel, { key: 'Tab', shiftKey: true }); expect(confirm).toHaveFocus(); fireEvent.keyDown(confirm, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); expect(opener).toHaveFocus(); expect(invoke).toHaveBeenCalledWith('cancel_media_preview', { requestId: expect.any(String), previewId: 'preview' });
  });
  it('discards a late preview after cancellation instead of re-opening or confirming it', async () => {
    let resolve!: (value: MediaImportPreview) => void;
    invoke.mockImplementation((command: string) => command === 'preview_media_import' ? new Promise(r => { resolve = r; }) : Promise.resolve(command === 'image_import_capabilities' ? { enabled: true } : command === 'choose_media_import' ? ['/a.png'] : command === 'image_import_jobs' ? [] : null));
    render(<Harness />); fireEvent.click(await screen.findByRole('button', { name: 'Choose media files' })); await screen.findByText('Checking selected files and folders…'); fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await act(async () => resolve(preview)); expect(screen.queryByRole('dialog')).not.toBeInTheDocument(); expect(invoke).toHaveBeenCalledWith('cancel_media_preview', { requestId: expect.any(String), previewId: 'preview' }); expect(invoke.mock.calls.some(([cmd]) => cmd === 'confirm_media_import')).toBe(false);
  });
  it('fences stale sequence/generation records and refreshes terminal failures immediately', async () => {
    expect(mergeImageJob([job], { ...job, sequence: '2', state: 'complete' })).toEqual([job]);
    expect(mergeImageJob([{ ...job, source_generation: '1' }], { ...job, id: 'old', sequence: '999' })).toEqual([{ ...job, source_generation: '1' }]);
    expect(mergeImageJob([job], { ...job, sequence: 'garbage' })).toEqual([job]);
    backendJobs = [job]; const { result } = renderHook(() => useImageImports(vi.fn(), vi.fn())); await waitFor(() => expect(result.current.jobs).toHaveLength(1));
    backendJobs = [{ ...job, sequence: '4', state: 'failed', error_code: 'DECODE_FAILED' }]; await act(async () => listeners.get('image-import-progress')?.({ payload: backendJobs[0] })); await waitFor(() => expect(result.current.jobs[0].state).toBe('failed'));
    backendJobs = [{ ...job, sequence: '1', state: 'cancelled' }]; await act(async () => listeners.get('image-import-reset')?.({ payload: null })); await waitFor(() => expect(result.current.jobs[0].sequence).toBe('1'));
    backendJobs = []; await act(async () => listeners.get('image-import-progress')?.({ payload: job })); await waitFor(() => expect(result.current.jobs).toHaveLength(0));
  });
  it('keeps metadata and cancellation usable with image development disabled', async () => {
    backendJobs = [job]; invoke.mockImplementation((command: string) => Promise.resolve(command === 'image_import_capabilities' ? { enabled: false } : command === 'image_import_jobs' ? backendJobs : command === 'cancel_image_import' ? { ...job, sequence: '4', state: 'cancelled' } : null));
    render(<Harness />); fireEvent.click(await screen.findByRole('button', { name: 'Cancel media import' })); await waitFor(() => expect(invoke).toHaveBeenCalledWith('cancel_image_import', { id: 'j1', sourceGeneration: '0' })); expect(screen.queryByText('Choose media files')).not.toBeInTheDocument(); expect(screen.queryByText('Retry media import')).not.toBeInTheDocument();
  });
  it('ignores raw renderer drop paths in image mode while preserving the legacy indicator', async () => {
    const legacyImport = vi.fn(); const { result } = renderHook(() => useDragDrop(legacyImport, vi.fn(), { ready: true, enabled: true })); await waitFor(() => expect(rawDrop).toBeDefined()); await act(async () => rawDrop?.({ payload: { type: 'over', paths: [] } })); expect(result.current.dropping).toBe(true); await act(async () => rawDrop?.({ payload: { type: 'drop', paths: ['/not-approved.png'] } })); expect(legacyImport).not.toHaveBeenCalled(); expect(result.current.dropping).toBe(false);
  });
});
it('browses durable image metadata after import history eviction with new imports disabled', async () => {
  invoke.mockImplementation((command: string) => Promise.resolve(command === 'image_import_capabilities' ? { enabled: false, metadata_present: true } : command === 'image_import_jobs' ? [] : command === 'imported_images' ? { items: [{ header: { id: 'image', relative_path: 'retained.png', technical_state: 'ready', availability: 'offline' }, width: 640, height: 480 }], next_after: null } : null));
  function Retained() { const c = useImageImports(vi.fn(), vi.fn()); return <ImageImports controller={c} sources={[{ id: 'source', name: 'Retained images', root: '/source', generation: 0, available: false, scope: 'folder', files: [] }]} />; }
  render(<Retained />); expect(await screen.findByText('retained.png')).toBeVisible(); expect(screen.getByRole('region', { name: 'Imported image library' })).toHaveTextContent('ready · offline · 640 × 480'); expect(screen.queryByText('Choose media files')).not.toBeInTheDocument(); expect(screen.queryByText('Browse files')).not.toBeInTheDocument();
});
