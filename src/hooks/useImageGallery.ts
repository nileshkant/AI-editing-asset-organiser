import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { imageGalleryPage, searchImages } from '../api';
import type { HitExplanation, ImageFacets, ImageQuery, ImageSort, ImageSummary, SearchInterpretation } from '../types';

export const GALLERY_PAGE = 100;
/** At most this many 100-item windows stay in memory (≈1,200 summaries). */
export const MAX_GALLERY_PAGES = 12;
type Page = { items: (ImageSummary | undefined)[]; generation: number };
/** Every SS-058 constraint except the source scope and sort, which the toolbar owns. */
export type ImageFilters = Omit<ImageQuery, 'version' | 'source_ids' | 'sort'>;
const STORAGE = 'creativeshelf.imageFilters.v1';
/** Typed words and tag names are private: only structural choices are remembered. */
const PRIVATE: (keyof ImageFilters)[] = ['text', 'tags', 'exclude_tags'];
const empty = (v: unknown) => v === undefined || v === null || v === '' || v === false || (Array.isArray(v) && !v.length);
export const activeFilters = (f: ImageFilters) => Object.entries(f).filter(([k, v]) => k !== 'tag_state' && k !== 'fields' && !empty(v)).length;
function remembered(): { filters: ImageFilters; sort: ImageSort } {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE) ?? 'null');
    if (raw && typeof raw === 'object') return { filters: raw.filters ?? {}, sort: raw.sort ?? 'relevance' };
  } catch { /* unavailable or corrupt storage: start clean */ }
  return { filters: {}, sort: 'relevance' };
}

/**
 * Windowed image metadata for a virtualized grid. Windows are fetched on demand,
 * fenced by a generation that changes with the query or catalog revision, and
 * evicted farthest-first. Selection is a set of asset IDs, independent of the
 * loaded windows, source filter or sort.
 */
export function useImageGallery(revision: string, reset: number) {
  const initial = useMemo(remembered, []);
  const [sourceId, setSourceId] = useState('');
  const [sort, setSort] = useState<ImageSort>(initial.sort);
  const [filters, setFilters] = useState<ImageFilters>(initial.filters);
  const [interpretation, setInterpretation] = useState<SearchInterpretation | null>(null);
  const [facets, setFacets] = useState<ImageFacets | null>(null);
  const explanations = useRef(new Map<string, HitExplanation>());
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState('');
  /** Changes whenever a window arrives; memoized views must depend on it. */
  const [version, setVersion] = useState(0);
  const [selected, setSelected] = useState<Map<string, ImageSummary>>(new Map());
  const pages = useRef(new Map<number, Page>()), inflight = useRef(new Map<number, number>());
  const generation = useRef(0), query = useRef(0), alive = useRef(true), wanted = useRef<[number, number]>([0, 0]);
  const bump = useCallback(() => setVersion(v => v + 1), []);
  const searchQuery: ImageQuery = useMemo(() => ({ ...filters, source_ids: sourceId ? [sourceId] : [], sort }), [filters, sourceId, sort]);
  const fetchPage = useCallback((page: number) => {
    const gen = generation.current, q = query.current;
    if (inflight.current.get(page) === gen) return;
    inflight.current.set(page, gen);
    const existing = pages.current.get(page);
    const items: (ImageSummary | undefined)[] = existing?.generation === gen ? [...existing.items] : [];
    const limit = (offset: number) => GALLERY_PAGE - (offset - page * GALLERY_PAGE);
    // Unfiltered name order is the gallery's indexed window; anything else is a search.
    const plain = !activeFilters(filters) && (sort === 'relevance' || sort === 'name_asc' || sort === 'name_desc');
    const request = (offset: number) => plain
      ? imageGalleryPage(sourceId || null, sort === 'name_desc' ? 'name_desc' : 'name_asc', offset, limit(offset))
      : searchImages(searchQuery, offset, limit(offset), false).then(r => {
        if (q === query.current) {
          r.explanations.forEach(e => explanations.current.set(e.id, e));
          if (offset === 0) setInterpretation(r.interpretation);
        }
        return r;
      });
    const fill = (offset: number): Promise<void> => request(offset).then(result => {
      if (!alive.current || q !== query.current || gen < generation.current) return;
      result.items.forEach((item, i) => { items[offset - page * GALLERY_PAGE + i] = item; });
      setTotal(result.total);
      const end = Math.min(result.total, (page + 1) * GALLERY_PAGE), next = offset + result.items.length;
      // A byte-capped short window resumes where it stopped.
      if (result.items.length && next < end) return fill(next);
    });
    void fill(page * GALLERY_PAGE).then(() => {
      if (!alive.current || q !== query.current || gen < generation.current) return;
      pages.current.set(page, { items, generation: gen });
      const [first, last] = wanted.current, centre = (first + last) / 2;
      while (pages.current.size > MAX_GALLERY_PAGES) {
        const far = [...pages.current.keys()].sort((a, b) => Math.abs(b - centre) - Math.abs(a - centre))[0];
        pages.current.delete(far);
      }
      setError(''); bump();
    }).catch(e => { if (alive.current && q === query.current) { setError(String(e)); bump(); } })
      .finally(() => { if (inflight.current.get(page) === gen) inflight.current.delete(page); });
  }, [sourceId, sort, filters, bump]); // eslint-disable-line react-hooks/exhaustive-deps
  /** Request every window overlapping [first, last] item indexes. */
  const ensure = useCallback((first: number, last: number) => {
    wanted.current = [Math.floor(first / GALLERY_PAGE), Math.floor(last / GALLERY_PAGE)];
    for (let p = wanted.current[0]; p <= wanted.current[1]; ++p) {
      if (pages.current.get(p)?.generation !== generation.current) fetchPage(p);
    }
  }, [fetchPage]);
  const item = useCallback((index: number) => pages.current.get(Math.floor(index / GALLERY_PAGE))?.items[index % GALLERY_PAGE], []);
  /** Catalog changed: refetch visible windows, keeping old ones visible meanwhile. */
  const refresh = useCallback(() => { ++generation.current; const [a, b] = wanted.current; for (let p = a; p <= b; ++p) fetchPage(p); }, [fetchPage]);
  useEffect(() => {
    ++query.current; ++generation.current; pages.current.clear(); inflight.current.clear(); explanations.current.clear();
    setTotal(null); setError(''); setInterpretation(null); setFacets(null); bump(); ensure(0, GALLERY_PAGE - 1);
  }, [sourceId, sort, filters, ensure, bump]);
  useEffect(() => {
    const kept = Object.fromEntries(Object.entries(filters).filter(([k]) => !PRIVATE.includes(k as keyof ImageFilters)));
    try { localStorage.setItem(STORAGE, JSON.stringify({ filters: kept, sort })); } catch { /* storage is optional */ }
  }, [filters, sort]);
  /** Counts for the whole matching set; requested on demand (the filter panel). */
  const loadFacets = useCallback(() => {
    const q = query.current;
    return searchImages(searchQuery, 0, 1, true).then(r => { if (alive.current && q === query.current) setFacets(r.facets); })
      .catch(e => { if (alive.current && q === query.current) setError(String(e)); });
  }, [sourceId, sort, filters]); // eslint-disable-line react-hooks/exhaustive-deps
  const lastRevision = useRef(revision);
  useEffect(() => {
    if (revision === lastRevision.current) return;
    lastRevision.current = revision;
    const timer = setTimeout(refresh, 400);
    return () => clearTimeout(timer);
  }, [revision, refresh]);
  useEffect(() => {
    if (!reset) return;
    // A restored database may not contain the selected identities.
    setSelected(new Map()); ++query.current; ++generation.current; pages.current.clear(); setTotal(null); bump(); ensure(0, GALLERY_PAGE - 1);
  }, [reset]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const toggle = useCallback((summary: ImageSummary) => setSelected(old => {
    const next = new Map(old);
    if (next.has(summary.header.id)) next.delete(summary.header.id); else next.set(summary.header.id, summary);
    return next;
  }), []);
  const explain = useCallback((id: string) => explanations.current.get(id), []);
  const resetFilters = useCallback(() => setFilters({}), []);
  return {
    version, sourceId, setSourceId, sort, setSort, total, error, item, ensure, refresh, selected, toggle, clearSelection: () => setSelected(new Map()),
    filters, setFilters, resetFilters, query: searchQuery, interpretation, facets, loadFacets, explain,
  };
}
export type ImageGalleryController = ReturnType<typeof useImageGallery>;
