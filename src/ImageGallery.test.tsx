import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useState } from 'react';
import { GALLERY_PAGE, MAX_GALLERY_PAGES, useImageGallery } from './hooks/useImageGallery';
import { ImageGrid } from './components/images/ImageGrid';
import { ImageInspector, transparencyLabel } from './components/images/ImageInspector';
import { ImageViewer } from './components/images/ImageViewer';
import { ImageGallery } from './components/images/ImageGallery';
import type { GalleryPage, ImageImportJob, ImageSummary, Source } from './types';
import type { ImageProfile } from './contracts/imageV1';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
const pad = (n: number) => String(n).padStart(12, '0');
const summary = (i: number, over: Partial<ImageSummary['header']> = {}): ImageSummary => ({
  header: { id: `00000000-0000-4000-8000-${pad(i)}`, kind: 'image', source_id: 's1', relative_path: `img-${pad(i)}.png`, content_hash: 'a'.repeat(64),
    technical_state: 'ready', availability: 'available', enrichment_state: 'disabled', ...over }, width: 800, height: 600, annotation_revision: 1 });
const page = (offset: number, limit: number, total: number, start = 0): GalleryPage => ({ offset, total, items: Array.from({ length: Math.max(0, Math.min(limit, total - offset)) }, (_, i) => summary(start + offset + i)) });
const profile = (over: Partial<ImageProfile> = {}): ImageProfile => ({ raw_width: 800, raw_height: 600, orientation: 1, width: 800, height: 600, format: 'png', encoded_bytes: 2048, bit_depth: 8, source_color_profile: null, alpha_channel: false, transparency: 'none', analyzer_version: 'v', ...over });
beforeEach(() => { invoke.mockReset(); URL.createObjectURL = vi.fn(() => 'blob:x'); URL.revokeObjectURL = vi.fn(); });
afterEach(cleanup);

describe('useImageGallery windows', () => {
  it('fetches windows on demand, fences a late reply after a query change, and bounds memory', async () => {
    const pending: { args: any; resolve: (p: GalleryPage) => void }[] = [];
    invoke.mockImplementation((cmd: string, args: any) => cmd === 'image_gallery_page' ? new Promise(resolve => pending.push({ args, resolve })) : Promise.resolve(null));
    const { result } = renderHook(() => useImageGallery('r0', 0));
    expect(pending[0].args).toEqual({ sourceId: null, sort: 'name_asc', offset: 0, limit: GALLERY_PAGE });
    act(() => result.current.setSort('name_desc'));
    // The old A–Z reply arrives after the sort change and is ignored.
    await act(async () => { pending[0].resolve(page(0, 100, 5000)); });
    expect(result.current.item(0)).toBeUndefined();
    await act(async () => { pending.at(-1)!.resolve(page(0, 100, 5000, 7)); });
    expect(result.current.item(0)?.header.id).toBe(summary(7).header.id);
    expect(result.current.total).toBe(5000);
    invoke.mockImplementation((cmd: string, args: any) => Promise.resolve(cmd === 'image_gallery_page' ? page(args.offset, args.limit, 5000) : null));
    for (let p = 1; p < 40; ++p) await act(async () => { result.current.ensure(p * GALLERY_PAGE, p * GALLERY_PAGE + 10); });
    const loaded = Array.from({ length: 40 }, (_, p) => result.current.item(p * GALLERY_PAGE)).filter(Boolean).length;
    expect(loaded).toBeLessThanOrEqual(MAX_GALLERY_PAGES);
    expect(result.current.item(39 * GALLERY_PAGE)).toBeDefined();
  });
  it('resumes a byte-capped short window and refreshes without dropping visible items', async () => {
    let calls = 0;
    invoke.mockImplementation((cmd: string, args: any) => {
      if (cmd !== 'image_gallery_page') return Promise.resolve(null);
      ++calls;
      // First reply stops early at 40 items, as the core 2 MiB bound may.
      return Promise.resolve(args.offset === 0 ? page(0, 40, 150) : page(args.offset, args.limit, 150));
    });
    const { result, rerender } = renderHook(({ rev }) => useImageGallery(rev, 0), { initialProps: { rev: 'r0' } });
    await waitFor(() => expect(result.current.item(99)).toBeDefined());
    expect(invoke).toHaveBeenCalledWith('image_gallery_page', { sourceId: null, sort: 'name_asc', offset: 40, limit: 60 });
    let release!: () => void;
    invoke.mockImplementation(() => new Promise(r => { release = () => r(page(0, 100, 151, 1)); }));
    vi.useFakeTimers();
    rerender({ rev: 'r1' });
    await act(async () => { vi.advanceTimersByTime(500); });
    vi.useRealTimers();
    // While the refresh is in flight the previous window stays visible.
    expect(result.current.item(0)?.header.id).toBe(summary(0).header.id);
    await act(async () => { release(); });
    await waitFor(() => expect(result.current.item(0)?.header.id).toBe(summary(1).header.id));
    expect(calls).toBeGreaterThan(1);
  });
  it('keeps selection by identity across source and sort changes, and clears it after a restore', async () => {
    invoke.mockImplementation((cmd: string, args: any) => Promise.resolve(cmd === 'image_gallery_page' ? page(args.offset, args.limit, 10) : null));
    const { result, rerender } = renderHook(({ reset }) => useImageGallery('r', reset), { initialProps: { reset: 0 } });
    await waitFor(() => expect(result.current.total).toBe(10));
    act(() => { result.current.toggle(summary(1)); result.current.toggle(summary(2)); });
    act(() => result.current.setSourceId('other'));
    act(() => result.current.setSort('name_desc'));
    expect([...result.current.selected.keys()]).toEqual([summary(1).header.id, summary(2).header.id]);
    act(() => result.current.toggle(summary(1)));
    expect(result.current.selected.size).toBe(1);
    rerender({ reset: 1 });
    expect(result.current.selected.size).toBe(0);
  });
});

describe('ImageGrid', () => {
  const props = (over = {}) => ({ version: 0, total: 10_000, cell: 144, item: (i: number) => summary(i), ensure: vi.fn(), focusIndex: 0, setFocusIndex: vi.fn(),
    selected: new Map<string, ImageSummary>(), activeId: null, onOpen: vi.fn(), onToggle: vi.fn(), onStale: vi.fn(), busy: false, ...over });
  it('renders only visible rows with grid semantics and bounded preview requests', () => {
    const p = props();
    render(<ImageGrid {...p} />);
    const grid = screen.getByRole('grid', { name: 'Images' });
    expect(grid).toHaveAttribute('aria-multiselectable', 'true');
    expect(grid).toHaveAttribute('aria-rowcount', String(Math.ceil(10_000 / 5)));
    const cells = screen.getAllByRole('gridcell');
    expect(cells.length).toBeLessThanOrEqual(5 * 7);
    expect(cells[0]).toHaveAttribute('tabindex', '0');
    expect(cells[1]).toHaveAttribute('tabindex', '-1');
    expect(cells[0]).toHaveAccessibleName('img-000000000000.png, 800 × 600');
    expect(p.ensure).toHaveBeenCalledWith(0, expect.any(Number));
  });
  it('moves by arrows, rows, Home/End and pages; Space selects and Enter opens', () => {
    const p = props();
    function Harness() { const [focus, setFocus] = useState(6); return <ImageGrid {...p} focusIndex={focus} setFocusIndex={setFocus} />; }
    render(<Harness />);
    const grid = screen.getByRole('grid');
    const focused = () => Number(document.activeElement?.getAttribute('data-index'));
    const key = (k: string, extra = {}) => fireEvent.keyDown(grid, { key: k, ...extra });
    key('ArrowRight'); expect(focused()).toBe(7);
    key('ArrowLeft'); expect(focused()).toBe(6);
    key('ArrowDown'); expect(focused()).toBe(11);
    key('ArrowUp'); expect(focused()).toBe(6);
    key('Home'); expect(focused()).toBe(5);
    key('End'); expect(focused()).toBe(9);
    key('End', { ctrlKey: true }); expect(focused()).toBe(9999);
    key('Home', { metaKey: true }); expect(focused()).toBe(0);
    key('PageDown'); expect(focused()).toBeGreaterThan(0);
    key('Home', { ctrlKey: true }); key('ArrowRight');
    key(' '); expect(p.onToggle).toHaveBeenCalledWith(summary(1));
    key('Enter'); expect(p.onOpen).toHaveBeenCalledWith(1, summary(1));
    fireEvent.click(screen.getAllByRole('gridcell')[2], { metaKey: true });
    expect(p.onToggle).toHaveBeenLastCalledWith(summary(2));
  });
  it('labels pending, failed and offline items and marks selection', () => {
    const items = [summary(0, { technical_state: 'pending', content_hash: null }), summary(1, { technical_state: 'failed' }), summary(2, { availability: 'offline' })];
    render(<ImageGrid {...props({ total: 3, item: (i: number) => items[i], selected: new Map([[items[2].header.id, items[2]]]) })} />);
    const cells = screen.getAllByRole('gridcell');
    expect(cells[0]).toHaveAccessibleName(/analysis pending/); expect(cells[0]).toHaveTextContent('Analyzing…');
    expect(cells[1]).toHaveAccessibleName(/technical analysis failed/); expect(cells[1]).toHaveTextContent('Analysis failed');
    expect(cells[2]).toHaveAccessibleName(/original offline/); expect(cells[2]).toHaveAttribute('aria-selected', 'true');
    // Only the ready item requests a preview, and only after the settle delay.
    expect(invoke).not.toHaveBeenCalledWith('image_preview', expect.anything());
  });
});

describe('inspector and viewer', () => {
  it('distinguishes channel support from actual transparency', () => {
    expect(transparencyLabel(profile())).toBe('No alpha channel · fully opaque');
    expect(transparencyLabel(profile({ alpha_channel: true }))).toBe('Alpha channel · every pixel opaque');
    expect(transparencyLabel(profile({ alpha_channel: true, transparency: 'binary' }))).toMatch(/^Binary transparency/);
    expect(transparencyLabel(profile({ alpha_channel: true, transparency: 'partial' }))).toMatch(/^Partial transparency/);
  });
  it('shows offline and failed states without a false ready preview, and Escape closes', async () => {
    const offline = summary(4, { availability: 'offline' });
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_details' ? { header: offline.header, profile: profile({ orientation: 6, raw_width: 600, raw_height: 800 }), source_name: 'Shoot', source_available: false, tags: ['hero'], caption: null, annotation_revision: 1 } : null));
    const onClose = vi.fn();
    render(<ImageInspector summary={offline} background="checkerboard" setBackground={vi.fn()} onClose={onClose} onStale={vi.fn()} />);
    expect(await screen.findByText('Shoot · offline')).toBeInTheDocument();
    expect(screen.getAllByText('Original offline · cached previews only, export disabled').length).toBeGreaterThan(0);
    expect(screen.getByText(/stored 600 × 800, EXIF orientation 6/)).toBeInTheDocument();
    expect(screen.getByText('Not enabled')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /export/i })).not.toBeInTheDocument();
    fireEvent.keyDown(screen.getByRole('complementary'), { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
    cleanup();
    const failed = summary(5, { technical_state: 'failed' });
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_details' ? { header: failed.header, profile: null, source_name: 'Shoot', source_available: true, tags: [], caption: null, annotation_revision: 1 } : null));
    render(<ImageInspector summary={failed} background="checkerboard" setBackground={vi.fn()} onClose={vi.fn()} onStale={vi.fn()} />);
    expect(await screen.findByText('Technical analysis failed; no preview is available.')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: /preview/ })).not.toBeInTheDocument();
  });
  it('zooms between fit and bounded 1:1 tiles', async () => {
    invoke.mockImplementation(() => new Promise(() => {}));
    render(<ImageViewer assetId={summary(1).header.id} version={'a'.repeat(64)} width={8000} height={6000} label="big.png" background="dark" onStale={vi.fn()} />);
    const tiles = () => invoke.mock.calls.filter(([c, a]) => c === 'image_preview' && a.tier === 'tile');
    expect(tiles()).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: '100%' }));
    await waitFor(() => expect(tiles().length).toBeGreaterThan(0));
    expect(tiles().length).toBeLessThanOrEqual(48);
    expect(screen.getByRole('region', { name: /big.png preview/ })).toHaveClass('bg-dark');
    fireEvent.keyDown(screen.getByRole('region', { name: /big.png preview/ }), { key: '0' });
    expect(screen.getByRole('button', { name: 'Fit' })).toHaveAttribute('aria-pressed', 'true');
  });
  it('does not request uncached 1:1 tiles when the original is offline', async () => {
    invoke.mockImplementation(() => new Promise(() => {}));
    render(<ImageViewer assetId={summary(1).header.id} version={'a'.repeat(64)} width={8000} height={6000} label="away.png" background="dark" onStale={vi.fn()} originalAvailable={false} />);
    fireEvent.click(screen.getByRole('button', { name: '100%' }));
    expect(await screen.findByText(/Full-resolution detail needs the original/)).toBeInTheDocument();
    expect(invoke.mock.calls.filter(([c, a]) => c === 'image_preview' && a.tier === 'tile')).toHaveLength(0);
  });
});

describe('ImageGallery states', () => {
  const controller = (over = {}) => ({ version: 0, sourceId: '', setSourceId: vi.fn(), sort: 'name_asc' as const, setSort: vi.fn(), total: 0, error: '', item: () => undefined,
    ensure: vi.fn(), refresh: vi.fn(), selected: new Map<string, ImageSummary>(), toggle: vi.fn(), clearSelection: vi.fn(), ...over });
  const source: Source = { id: 's1', name: 'Shoot', root: '/shoot', generation: 0, available: true, scope: 'folder', files: [] };
  const job = (over: Partial<ImageImportJob>): ImageImportJob => ({ id: 'j', source_id: 's1', source_generation: '0', sequence: '1', mode: 'images', state: 'running', phase: 'analysis', discovery_complete: true, history_truncated: false, total: 10, completed: 4, ready: 3, reused: 0, failed: 0, unsupported: 0, current: '', error_code: null, ...over });
  it('explains no sources, empty views, running imports and partial failures with actions', () => {
    const open = vi.fn();
    const { rerender } = render(<ImageGallery gallery={controller()} sources={[]} jobs={[]} onOpenImports={open} />);
    expect(screen.getByRole('heading', { name: 'No sources yet' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Go to Imports' })); expect(open).toHaveBeenCalled();
    rerender(<ImageGallery gallery={controller()} sources={[source]} jobs={[]} onOpenImports={open} />);
    expect(screen.getByText(/No supported images \(JPEG, PNG or WebP\)/)).toBeInTheDocument();
    rerender(<ImageGallery gallery={controller()} sources={[source]} jobs={[job({}), job({ id: 'k', state: 'failed', failed: 2 })]} onOpenImports={open} />);
    expect(screen.getByRole('heading', { name: 'Import in progress' })).toBeInTheDocument();
    expect(screen.getByText(/Importing images: 4 of 10 files checked/)).toBeInTheDocument();
    expect(screen.getByText(/2 files could not be imported/)).toBeInTheDocument();
    rerender(<ImageGallery gallery={controller({ total: null })} sources={[source]} jobs={[]} onOpenImports={open} />);
    expect(screen.getAllByText('Loading images…').length).toBeGreaterThan(0);
    rerender(<ImageGallery gallery={controller({ error: 'INTERNAL' })} sources={[source]} jobs={[]} onOpenImports={open} />);
    expect(screen.getByRole('alert')).toHaveTextContent('Images could not be listed');
  });
  it('enables comparison only for 2–4 selected images', () => {
    const sel = (n: number) => new Map(Array.from({ length: n }, (_, i) => [summary(i).header.id, summary(i)]));
    const { rerender } = render(<ImageGallery gallery={controller({ total: 5, selected: sel(1), item: (i: number) => summary(i) })} sources={[source]} jobs={[]} onOpenImports={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Compare selected' })).toBeDisabled();
    rerender(<ImageGallery gallery={controller({ total: 5, selected: sel(3), item: (i: number) => summary(i) })} sources={[source]} jobs={[]} onOpenImports={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Compare selected' })).toBeEnabled();
    expect(screen.getByText('3 selected')).toBeInTheDocument();
    rerender(<ImageGallery gallery={controller({ total: 5, selected: sel(5), item: (i: number) => summary(i) })} sources={[source]} jobs={[]} onOpenImports={vi.fn()} />);
    expect(screen.getByRole('button', { name: 'Compare selected' })).toBeDisabled();
  });
});
