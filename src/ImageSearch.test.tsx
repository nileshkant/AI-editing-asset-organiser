import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useImageGallery, type ImageGalleryController } from './hooks/useImageGallery';
import { ImageSearchBar, NoMatches, describeSearch } from './components/images/ImageSearchBar';
import type { ImageSearchPage, ImageSummary, SearchInterpretation } from './types';
const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: (...args: unknown[]) => invoke(...args) }));
const pad = (n: number) => String(n).padStart(12, '0');
const summary = (i: number): ImageSummary => ({
  header: { id: `00000000-0000-4000-8000-${pad(i)}`, kind: 'image', source_id: 's1', relative_path: `img-${pad(i)}.png`, content_hash: 'a'.repeat(64),
    technical_state: 'ready', availability: 'available', enrichment_state: 'disabled' }, width: 800, height: 600, annotation_revision: 1 });
const interpretation = (over: Partial<SearchInterpretation> = {}): SearchInterpretation => ({ clauses: [], fields: ['name', 'tags', 'caption', 'notes'], tags: [], exclude_tags: [], ignored: [], suggestions: [], text_only_matches: null, ...over });
const result = (offset: number, limit: number, total: number, over: Partial<ImageSearchPage> = {}): ImageSearchPage => {
  const items = Array.from({ length: Math.max(0, Math.min(limit, total - offset)) }, (_, i) => summary(offset + i));
  return { items, explanations: items.map(i => ({ id: i.header.id, tier: 1, fields: ['tags'] })), offset, total, interpretation: interpretation(), facets: null, ...over };
};
const caps = [{ field: 'name', available: true, owner: null }, { field: 'tags', available: true, owner: null }, { field: 'suggested_tags', available: true, owner: null },
  { field: 'caption', available: true, owner: null }, { field: 'notes', available: true, owner: null }, { field: 'ocr', available: false, owner: 'SS-057' }];
beforeEach(() => { localStorage.clear(); invoke.mockReset(); });
afterEach(cleanup);

describe('useImageGallery search', () => {
  it('keeps the indexed gallery path until a filter applies, then searches with the canonical query', async () => {
    invoke.mockImplementation((cmd: string, args: any) => Promise.resolve(
      cmd === 'image_gallery_page' ? { items: [summary(0)], offset: 0, total: 1 } : cmd === 'search_images' ? result(args.offset, args.limit, 3) : null));
    const { result: h } = renderHook(() => useImageGallery('r', 0));
    await waitFor(() => expect(h.current.total).toBe(1));
    expect(invoke).toHaveBeenCalledWith('image_gallery_page', { sourceId: null, sort: 'name_asc', offset: 0, limit: 100 });
    act(() => { h.current.setSourceId('s1'); h.current.setFilters({ text: 'sunset', min_rating: 4, tags: ['beach'] }); });
    await waitFor(() => expect(h.current.total).toBe(3));
    expect(invoke).toHaveBeenLastCalledWith('search_images', { query: { text: 'sunset', min_rating: 4, tags: ['beach'], source_ids: ['s1'], sort: 'relevance' }, offset: 0, limit: 100, facets: false });
    expect(h.current.explain(summary(1).header.id)).toEqual({ id: summary(1).header.id, tier: 1, fields: ['tags'] });
    expect(h.current.interpretation).toEqual(interpretation());
    // Structural choices persist; typed words and tag names do not.
    expect(JSON.parse(localStorage.getItem('creativeshelf.imageFilters.v1')!)).toEqual({ filters: { min_rating: 4 }, sort: 'relevance' });
    invoke.mockImplementation((cmd: string, args: any) => Promise.resolve(cmd === 'search_images' ? result(0, 1, 3, { facets: { tags: [{ value: 'beach', count: 3 }], suggested_tags: [], orientations: [], transparency: [], ratings: [], formats: [], availability: [], technical: [], sources: [], favorites: 0, needs_review: 0 } }) : null));
    await act(async () => { await h.current.loadFacets(); });
    expect(invoke).toHaveBeenLastCalledWith('search_images', expect.objectContaining({ offset: 0, limit: 1, facets: true }));
    expect(h.current.facets?.tags).toEqual([{ value: 'beach', count: 3 }]);
    act(() => h.current.resetFilters());
    await waitFor(() => expect(h.current.total).toBe(null));
  });
  it('restores remembered structural filters and survives unavailable storage', async () => {
    localStorage.setItem('creativeshelf.imageFilters.v1', JSON.stringify({ filters: { ready_only: true }, sort: 'rating_desc' }));
    invoke.mockImplementation((cmd: string, args: any) => Promise.resolve(cmd === 'search_images' ? result(args.offset, args.limit, 0) : null));
    const { result: h } = renderHook(() => useImageGallery('r', 0));
    expect(h.current.filters).toEqual({ ready_only: true });
    expect(h.current.sort).toBe('rating_desc');
    localStorage.setItem('creativeshelf.imageFilters.v1', '{not json');
    const { result: h2 } = renderHook(() => useImageGallery('r', 0));
    expect(h2.current.filters).toEqual({});
  });
});

const controller = (over: Partial<ImageGalleryController> = {}): ImageGalleryController => ({
  version: 0, sourceId: '', setSourceId: vi.fn(), sort: 'relevance', setSort: vi.fn(), total: 0, error: '', item: () => undefined, ensure: vi.fn(), refresh: vi.fn(),
  selected: new Map(), toggle: vi.fn(), clearSelection: vi.fn(), filters: {}, setFilters: vi.fn(), resetFilters: vi.fn(), query: { source_ids: [], sort: 'relevance' },
  interpretation: null, facets: null, loadFacets: vi.fn(() => Promise.resolve()), explain: () => undefined, ...over,
} as ImageGalleryController);

describe('ImageSearchBar', () => {
  beforeEach(() => {
    invoke.mockImplementation((cmd: string) => Promise.resolve(cmd === 'image_search_capabilities' ? caps
      : cmd === 'saved_image_searches' ? [{ id: 'a', name: 'Beach picks', query: { version: 1, text: 'beach', source_ids: ['s2'], sort: 'rating_desc', min_rating: 4 }, created_at: 1, updated_at: 1 },
        { id: 'b', name: 'Future', query: null, created_at: 1, updated_at: 1 }] : null));
  });
  it('applies typed text after a pause and offers only implemented fields', async () => {
    const setFilters = vi.fn();
    render(<ImageSearchBar gallery={controller({ setFilters })} />);
    vi.useFakeTimers();
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search images' }), { target: { value: '"golden hour" -night' } });
    expect(setFilters).not.toHaveBeenCalled();
    await act(async () => { vi.advanceTimersByTime(350); });
    vi.useRealTimers();
    expect(setFilters.mock.calls[0][0]({ min_rating: 2 })).toEqual({ min_rating: 2, text: '"golden hour" -night' });
    fireEvent.click(screen.getByRole('button', { name: 'Filters' }));
    await waitFor(() => expect(screen.getByLabelText('Suggested tags')).toBeInTheDocument());
    expect(screen.queryByLabelText('Text in images')).toBeNull();
    expect(screen.getByText(/never match a size or shape filter/)).toBeInTheDocument();
  });
  it('opens saved searches, reports newer-version ones and resolves name conflicts explicitly', async () => {
    const setFilters = vi.fn(), setSourceId = vi.fn(), setSort = vi.fn();
    render(<ImageSearchBar gallery={controller({ setFilters, setSourceId, setSort, filters: { text: 'beach' } })} />);
    await waitFor(() => expect(screen.getByRole('option', { name: 'Beach picks' })).toBeInTheDocument());
    fireEvent.change(screen.getByRole('combobox', { name: 'Saved searches' }), { target: { value: 'a' } });
    expect(setFilters).toHaveBeenCalledWith({ text: 'beach', min_rating: 4 });
    expect(setSourceId).toHaveBeenCalledWith('s2');
    expect(setSort).toHaveBeenCalledWith('rating_desc');
    fireEvent.change(screen.getByRole('combobox', { name: 'Saved searches' }), { target: { value: 'b' } });
    expect(screen.getByRole('status')).toHaveTextContent('saved by a newer version');
    invoke.mockImplementation((cmd: string) => cmd === 'save_image_search' ? Promise.reject('CONFLICT') : Promise.resolve([]));
    fireEvent.click(screen.getByRole('button', { name: 'Save search…' }));
    fireEvent.change(screen.getByRole('textbox', { name: 'Saved search name' }), { target: { value: 'Beach picks' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('Choose Replace'));
    invoke.mockImplementation((cmd: string, args: any) => Promise.resolve(cmd === 'save_image_search' ? { id: 'a', name: args.name, query: args.query, created_at: 1, updated_at: 2 } : []));
    fireEvent.click(screen.getByRole('button', { name: 'Replace existing' }));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('Saved “Beach picks”'));
    expect(invoke).toHaveBeenCalledWith('save_image_search', { name: 'Beach picks', query: { source_ids: [], sort: 'relevance', text: 'beach' }, replace: true });
  });
  it('describes the interpretation and guides a search with no matches', () => {
    const i = interpretation({ clauses: [{ text: 'sunsett', kind: 'term', excluded: false, matches: 0 }, { text: 'night', kind: 'term', excluded: true, matches: 4 }],
      tags: [{ requested: 'nyc', tag: 'New York', via_alias: true }], suggestions: [{ term: 'sunsett', candidates: ['sunset'] }], text_only_matches: 2, ignored: ['--'] });
    expect(describeSearch(i)).toBe('Matching sunsett in file names, tags, captions, notes · without night · tag New York (alias nyc)');
    const setFilters = vi.fn(), resetFilters = vi.fn(), onText = vi.fn();
    render(<NoMatches gallery={controller({ interpretation: i, filters: { text: 'sunsett -night' }, setFilters, resetFilters })} onText={onText} />);
    expect(screen.getByText(/No image contains “sunsett”/)).toBeInTheDocument();
    expect(screen.getByText(/2 images match your words but not the other filters/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'sunset' }));
    expect(onText).toHaveBeenCalledWith('sunset -night');
    fireEvent.click(screen.getByRole('button', { name: 'Clear search and filters' }));
    expect(resetFilters).toHaveBeenCalled();
  });
});
