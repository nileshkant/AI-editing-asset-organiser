import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { parsePreviewEnvelope } from './api';
import { ImagePreview } from './components/image/ImagePreview';
import { ImagePreviewSettings } from './components/views/ImagePreviewSettings';
import type { ImagePreviewCacheStatus, ImagePreviewMeta } from './types';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
const id = '7b0f8d2e-41a4-4c55-9d1e-0c6c6f1f0a11', v1 = 'a'.repeat(64), v2 = 'b'.repeat(64);
const meta = (over: Partial<ImagePreviewMeta> = {}): ImagePreviewMeta => ({ asset_id: id, version: v1, tier: 'small', tile: null, width: 128, height: 96, source_width: 640, source_height: 480, mime: 'image/png', transparency: false, cache: 'hit', original_available: true, offline_reference: false, preview_version: 'creativeshelf-preview-v1', ...over });
function envelope(m: ImagePreviewMeta, payload = [0x89, 0x50, 0x4e, 0x47]) {
  const json = new TextEncoder().encode(JSON.stringify(m)), out = new Uint8Array(8 + json.length + payload.length);
  out.set([0x43, 0x53, 0x50, 0x56]); new DataView(out.buffer).setUint32(4, json.length, true); out.set(json, 8); out.set(payload, 8 + json.length);
  return out.buffer;
}
const created: string[] = [], revoked: string[] = [];
beforeEach(() => {
  invoke.mockReset(); created.length = 0; revoked.length = 0;
  URL.createObjectURL = vi.fn(() => { const url = `blob:preview-${created.length}`; created.push(url); return url; });
  URL.revokeObjectURL = vi.fn((url: string) => { revoked.push(url); });
});
afterEach(cleanup);
const target = { assetId: id, version: v1, tier: 'small' as const, tile: null };

describe('preview envelope contract', () => {
  it('accepts only the requested asset version and tier with bounded metadata', () => {
    const { meta: m, blob } = parsePreviewEnvelope(envelope(meta()), target);
    expect(m.width).toBe(128); expect(blob.type).toBe('image/png'); expect(blob.size).toBe(4);
    expect(() => parsePreviewEnvelope(envelope(meta({ version: v2 })), target)).toThrow('INVALID_DATA');
    expect(() => parsePreviewEnvelope(envelope(meta({ asset_id: 'other' })), target)).toThrow('INVALID_DATA');
    expect(() => parsePreviewEnvelope(envelope(meta({ tier: 'inspect' })), target)).toThrow('INVALID_DATA');
    expect(() => parsePreviewEnvelope(envelope(meta({ tile: [0, 0] })), target)).toThrow('INVALID_DATA');
    expect(() => parsePreviewEnvelope(envelope(meta({ mime: 'text/html' as 'image/png' })), target)).toThrow('INVALID_DATA');
    expect(() => parsePreviewEnvelope(envelope(meta({ offline_reference: true })), target)).toThrow('INVALID_DATA');
    expect(() => parsePreviewEnvelope(envelope(meta(), []), target)).toThrow('INVALID_DATA');
    const bad = new Uint8Array(envelope(meta())); bad[0] = 0; expect(() => parsePreviewEnvelope(bad.buffer, target)).toThrow('INVALID_DATA');
    const huge = new Uint8Array(envelope(meta())); new DataView(huge.buffer).setUint32(4, 1 << 20, true); expect(() => parsePreviewEnvelope(huge, target)).toThrow('INVALID_DATA');
  });
});

describe('ImagePreview', () => {
  it('requests by ID/version only, fences a stale reply after a version change, and revokes owned URLs', async () => {
    const pending: ((value: ArrayBuffer) => void)[] = [];
    invoke.mockImplementation((command: string, args: { version: string }) => command === 'image_preview' ? new Promise(r => { pending.push(r); }) : Promise.resolve(true));
    const { rerender, unmount } = render(<ImagePreview assetId={id} version={v1} tier="small" label="cat.png" />);
    expect(screen.getByRole('status', { name: 'Loading preview of cat.png' })).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('image_preview', { requestId: expect.any(String), assetId: id, version: v1, tier: 'small', tile: null });
    const first = invoke.mock.calls[0][1].requestId;
    rerender(<ImagePreview assetId={id} version={v2} tier="small" label="cat.png" />);
    expect(invoke).toHaveBeenCalledWith('cancel_image_preview', { requestId: first });
    await act(async () => { pending[1](envelope(meta({ version: v2, width: 64, height: 48 }))); });
    const img = await screen.findByRole('img');
    expect(img).toHaveAttribute('src', 'blob:preview-0'); expect(img).toHaveAttribute('width', '64');
    // The obsolete v1 reply arrives late and is ignored without creating a URL.
    await act(async () => { pending[0](envelope(meta())); });
    expect(created).toEqual(['blob:preview-0']); expect(screen.getByRole('img')).toHaveAttribute('width', '64');
    unmount();
    expect(revoked).toEqual(['blob:preview-0']);
    expect(invoke.mock.calls.filter(([c]) => c === 'cancel_image_preview')).toHaveLength(1);
  });
  it('labels offline cached references and transparent previews without implying export', async () => {
    invoke.mockResolvedValue(envelope(meta({ original_available: false, offline_reference: true, transparency: true })));
    render(<ImagePreview assetId={id} version={v1} tier="small" label="logo.png" />);
    const img = await screen.findByRole('img', { name: 'logo.png, 640 × 480, transparent' });
    expect(img).toHaveClass('checkerboard');
    expect(screen.getByText('Offline reference · original unavailable, export disabled')).toBeInTheDocument();
  });
  it('shows explicit stale, unavailable and retryable states', async () => {
    invoke.mockRejectedValueOnce('ORIGINAL_UNAVAILABLE');
    const { unmount } = render(<ImagePreview assetId={id} version={v1} tier="small" label="gone.png" />);
    expect(await screen.findByText('Original unavailable · no cached preview')).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument(); unmount();
    invoke.mockRejectedValueOnce('STALE_VERSION').mockResolvedValueOnce(envelope(meta()));
    render(<ImagePreview assetId={id} version={v1} tier="small" label="cat.png" />);
    expect(await screen.findByText('Preview out of date · the image changed')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry preview of cat.png' }));
    expect(await screen.findByRole('img')).toBeInTheDocument();
  });
});

describe('ImagePreviewSettings', () => {
  const status = (over: Partial<ImagePreviewCacheStatus> = {}): ImagePreviewCacheStatus => ({ enabled: true, entries: 12, bytes: 3 * 1024 * 1024, quota_bytes: 512 * 1024 * 1024, active: 0, waiting: 0, counters: { hits: 30, misses: 12, generated: 4, coalesced: 1, evicted: 2, repaired: 1, write_failures: 0, cancelled: 3, stale_rejected: 0, offline_served: 5 }, ...over });
  it('stays hidden when previews are unavailable and nothing is cached', async () => {
    invoke.mockResolvedValue(status({ enabled: false, entries: 0, bytes: 0 }));
    const { container } = render(<ImagePreviewSettings onError={vi.fn()} />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('image_preview_cache_status', undefined));
    expect(container).toBeEmptyDOMElement();
  });
  it('shows counts only, persists the quota and clears only after confirmation', async () => {
    invoke.mockImplementation((command: string, args?: { quotaBytes: number }) => Promise.resolve(
      command === 'image_preview_cache_status' ? status() : command === 'set_image_preview_cache_quota' ? status({ quota_bytes: args!.quotaBytes }) : command === 'clear_image_preview_cache' ? 12 : null));
    render(<ImagePreviewSettings onError={vi.fn()} />);
    expect(await screen.findByText(/12 previews · 3 MiB of 512 MiB/)).toBeInTheDocument();
    expect(screen.getByText(/30 cache hits · 4 generated · 2 evicted · 1 repaired · 0 write failures · 5 offline references/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole('combobox', { name: 'Preview cache size' }), { target: { value: String(1024 * 1024 * 1024) } });
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('set_image_preview_cache_quota', { quotaBytes: 1024 * 1024 * 1024 }));
    expect(await screen.findByText(/limited to 1 GiB/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Clear image previews' }));
    expect(invoke).not.toHaveBeenCalledWith('clear_image_preview_cache', expect.anything());
    expect(screen.getByRole('group', { name: 'Confirm preview cache clear' })).toHaveTextContent('technical analysis, annotations and tags stay intact');
    fireEvent.click(screen.getByRole('button', { name: 'Confirm preview clear' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('clear_image_preview_cache', { confirmed: true }));
    expect(await screen.findByText('Removed 12 image previews. They rebuild on demand.')).toBeInTheDocument();
  });
});
