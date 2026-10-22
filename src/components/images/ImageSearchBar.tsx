import { useEffect, useId, useState } from 'react';
import { deleteSavedImageSearch, imageSearchCapabilities, saveImageSearch, savedImageSearches } from '../../api';
import { activeFilters, type ImageFilters, type ImageGalleryController } from '../../hooks/useImageGallery';
import type { FacetCount, FieldCapability, ImageQuery, Orientation, SavedImageSearch, SearchInterpretation, TextField } from '../../types';

export const FIELD_LABELS: Record<TextField, string> = {
  name: 'File names', tags: 'Tags', suggested_tags: 'Suggested tags', caption: 'Captions', notes: 'Notes',
  ocr: 'Text in images', generation: 'Generation details', usage_notes: 'Usage notes',
};
const ORIENTATIONS: [Orientation, string][] = [['landscape', 'Landscape'], ['portrait', 'Portrait'], ['square', 'Square']];
const TRANSPARENCY: ['none' | 'binary' | 'partial', string][] = [['none', 'Opaque'], ['binary', 'Cut-out'], ['partial', 'Soft transparency']];
const count = (list: FacetCount[] | undefined, value: string) => list?.find(f => f.value === value)?.count;
const withCount = (label: string, n: number | undefined) => n === undefined ? label : `${label} (${n.toLocaleString()})`;
const splitTags = (text: string) => text.split(',').map(t => t.trim()).filter(Boolean);
const positive = (v: string) => { const n = Number(v); return v.trim() && Number.isInteger(n) && n > 0 ? n : null; };
const toggle = <T,>(list: T[] | undefined, value: T) => list?.includes(value) ? list.filter(v => v !== value) : [...(list ?? []), value];

/** One-line description of what a search is matching, from the core's interpretation. */
export function describeSearch(i: SearchInterpretation | null): string {
  if (!i) return '';
  const parts: string[] = [];
  const terms = i.clauses.filter(c => !c.excluded).map(c => c.kind === 'phrase' ? `“${c.text}”` : c.kind === 'prefix' ? `${c.text}…` : c.text);
  if (terms.length) parts.push(`${terms.join(' and ')} in ${i.fields.map(f => FIELD_LABELS[f].toLowerCase()).join(', ')}`);
  const excluded = i.clauses.filter(c => c.excluded).map(c => c.text);
  if (excluded.length) parts.push(`without ${excluded.join(', ')}`);
  for (const t of i.tags) parts.push(`tag ${t.tag}${t.via_alias ? ` (alias ${t.requested})` : ''}`);
  for (const t of i.exclude_tags) parts.push(`not tagged ${t.tag}`);
  return parts.length ? `Matching ${parts.join(' · ')}` : '';
}

type Props = { gallery: ImageGalleryController };
/**
 * SS-058 exact search controls. Text is applied after a short pause; every
 * other control applies at once. Nothing here relaxes a filter on its own.
 */
export function ImageSearchBar({ gallery: g }: Props) {
  const id = useId();
  const [text, setText] = useState(g.filters.text ?? '');
  const [open, setOpen] = useState(false);
  const [fields, setFields] = useState<FieldCapability[]>([]);
  const [saved, setSaved] = useState<SavedImageSearch[]>([]);
  const [saving, setSaving] = useState<string | null>(null);
  const [notice, setNotice] = useState('');
  const set = (patch: Partial<ImageFilters>) => g.setFilters(f => ({ ...f, ...patch }));
  useEffect(() => { setText(g.filters.text ?? ''); }, [g.filters.text]);
  useEffect(() => {
    if (text === (g.filters.text ?? '')) return;
    const timer = setTimeout(() => g.setFilters(f => ({ ...f, text })), 300);
    return () => clearTimeout(timer);
  }, [text]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { imageSearchCapabilities().then(setFields).catch(() => setFields([])); }, []);
  const reloadSaved = () => savedImageSearches().then(setSaved).catch(e => setNotice(String(e)));
  useEffect(() => { void reloadSaved(); }, []);
  useEffect(() => { if (open) void g.loadFacets(); }, [open, g.loadFacets]); // eslint-disable-line react-hooks/exhaustive-deps
  const f = g.filters, facets = g.facets;
  const searched = f.fields?.length ? f.fields : ['name', 'tags', 'caption', 'notes'] as TextField[];
  const n = activeFilters(f);
  const apply = (s: SavedImageSearch) => {
    if (!s.query) { setNotice(`“${s.name}” was saved by a newer version of CreativeShelf and can't be opened here.`); return; }
    const { version: _v, source_ids, sort, ...rest } = s.query as ImageQuery;
    g.setFilters(rest); g.setSourceId(source_ids?.[0] ?? ''); if (sort) g.setSort(sort);
    setNotice(`Opened “${s.name}”.`);
  };
  const save = async (replace: boolean) => {
    if (!saving?.trim()) return;
    try {
      await saveImageSearch(saving, { ...g.query, text }, replace);
      setNotice(`Saved “${saving.trim()}”.`); setSaving(null); await reloadSaved();
    } catch (e) {
      const code = String(e);
      setNotice(code.includes('CONFLICT') ? 'A saved search with this name exists. Choose Replace to update it.' : `Could not save: ${code}`);
      if (!code.includes('CONFLICT')) setSaving(null);
    }
  };
  return <div className="image-search">
    <div className="image-search-row">
      <input type="search" aria-label="Search images" aria-describedby={`${id}-help`} placeholder="Search names, tags, captions and notes"
        title='Whole words. "quotes" for a phrase, -word to exclude, word* for a prefix.'
        value={text} maxLength={512} onChange={e => setText(e.target.value)}
        onKeyDown={e => { if (e.key === 'Enter') g.setFilters(x => ({ ...x, text })); if (e.key === 'Escape' && text) { setText(''); g.setFilters(x => ({ ...x, text: '' })); } }} />
      <button aria-expanded={open} aria-controls={`${id}-panel`} onClick={() => setOpen(o => !o)}>Filters{n ? ` (${n})` : ''}</button>
      <button disabled={!n} onClick={() => { setText(''); g.resetFilters(); setNotice('Search and filters cleared.'); }}>Clear all</button>
      <select aria-label="Saved searches" value="" onChange={e => { const s = saved.find(x => x.id === e.target.value); if (s) apply(s); }}>
        <option value="">Saved searches…</option>
        {saved.map(s => <option key={s.id} value={s.id}>{s.name}{s.query ? '' : ' (newer version)'}</option>)}
      </select>
      <button disabled={!n} onClick={() => setSaving('')}>Save search…</button>
    </div>
    <p className="sr-only" id={`${id}-help`}>Words match whole words. Use "quotes" for a phrase, -word to exclude, word* for a prefix.</p>
    {saving !== null && <div className="image-search-save" role="group" aria-label="Save search">
      <input aria-label="Saved search name" autoFocus value={saving} maxLength={256} onChange={e => setSaving(e.target.value)} onKeyDown={e => { if (e.key === 'Enter') void save(false); if (e.key === 'Escape') setSaving(null); }} />
      <button onClick={() => void save(false)} disabled={!saving.trim()}>Save</button>
      <button onClick={() => void save(true)} disabled={!saving.trim()}>Replace existing</button>
      <button onClick={() => setSaving(null)}>Cancel</button>
    </div>}
    {notice && <p className="image-search-notice" role="status">{notice}</p>}
    {open && <div className="image-filter-panel" id={`${id}-panel`} role="group" aria-label="Image filters">
      <fieldset><legend>Search in</legend>
        {fields.filter(c => c.available).map(c => <label key={c.field}><input type="checkbox" checked={searched.includes(c.field)}
          disabled={searched.length === 1 && searched.includes(c.field)}
          onChange={() => set({ fields: toggle(searched, c.field) })} /> {FIELD_LABELS[c.field]}</label>)}
      </fieldset>
      <fieldset><legend>Tags</legend>
        <label>Has all of <input aria-label="Required tags" placeholder="comma separated" defaultValue={(f.tags ?? []).join(', ')} key={`t${(f.tags ?? []).join()}`}
          onBlur={e => set({ tags: splitTags(e.target.value) })} /></label>
        <label>Has none of <input aria-label="Excluded tags" placeholder="comma separated" defaultValue={(f.exclude_tags ?? []).join(', ')} key={`x${(f.exclude_tags ?? []).join()}`}
          onBlur={e => set({ exclude_tags: splitTags(e.target.value) })} /></label>
        <label>Tag source <select value={f.tag_state ?? 'confirmed'} onChange={e => set({ tag_state: e.target.value as ImageFilters['tag_state'] })}>
          <option value="confirmed">Confirmed tags</option><option value="suggested">Suggestions awaiting review</option><option value="any">Confirmed or suggested</option>
        </select></label>
        {!!facets?.tags.length && <p className="facet-list" aria-label="Common tags">{facets.tags.slice(0, 12).map(t =>
          <button key={t.value} className="chip" onClick={() => set({ tags: [...new Set([...(f.tags ?? []), t.value])] })}>{t.value} {t.count.toLocaleString()}</button>)}</p>}
      </fieldset>
      <fieldset><legend>Rating and favorites</legend>
        <label>Favorites <select value={f.favorite == null ? '' : String(f.favorite)} onChange={e => set({ favorite: e.target.value === '' ? null : e.target.value === 'true' })}>
          <option value="">Any</option><option value="true">{withCount('Favorites only', facets?.favorites)}</option><option value="false">Not favorites</option>
        </select></label>
        <label>Rating <select value={f.rated === false ? 'unrated' : String(f.min_rating ?? '')}
          onChange={e => e.target.value === 'unrated' ? set({ rated: false, min_rating: null }) : set({ rated: null, min_rating: e.target.value ? Number(e.target.value) : null })}>
          <option value="">Any</option>
          {[5, 4, 3, 2, 1].map(r => <option key={r} value={r}>{withCount(`${'★'.repeat(r)}${r < 5 ? ' or more' : ''}`, r === 5 ? count(facets?.ratings, '5') : undefined)}</option>)}
          <option value="unrated">{withCount('Unrated', count(facets?.ratings, 'unrated'))}</option>
        </select></label>
      </fieldset>
      <fieldset><legend>Shape and size</legend>
        {ORIENTATIONS.map(([o, label]) => <label key={o}><input type="checkbox" checked={!!f.orientations?.includes(o)} onChange={() => set({ orientations: toggle(f.orientations, o) })} /> {withCount(label, count(facets?.orientations, o))}</label>)}
        <label>Min width <input inputMode="numeric" aria-label="Minimum width in pixels" defaultValue={f.min_width ?? ''} key={`w${f.min_width}`} onBlur={e => set({ min_width: positive(e.target.value) })} /></label>
        <label>Min height <input inputMode="numeric" aria-label="Minimum height in pixels" defaultValue={f.min_height ?? ''} key={`h${f.min_height}`} onBlur={e => set({ min_height: positive(e.target.value) })} /></label>
        <small>Images without measured dimensions never match a size or shape filter.</small>
      </fieldset>
      <fieldset><legend>Transparency</legend>
        {TRANSPARENCY.map(([t, label]) => <label key={t}><input type="checkbox" checked={!!f.transparency?.includes(t)} onChange={() => set({ transparency: toggle(f.transparency, t) })} /> {withCount(label, count(facets?.transparency, t))}</label>)}
      </fieldset>
      <fieldset><legend>Status</legend>
        <label><input type="checkbox" checked={!!f.ready_only} onChange={e => set({ ready_only: e.target.checked })} /> Ready to use (analyzed and available)</label>
        <label><input type="checkbox" checked={f.review === 'needs_review'} onChange={e => set({ review: e.target.checked ? 'needs_review' : null })} /> {withCount('Has suggestions to review', facets?.needs_review)}</label>
      </fieldset>
    </div>}
  </div>;
}

/** No-match guidance from the core: per-word counts, near words and filter conflicts. */
export function NoMatches({ gallery: g, onText }: { gallery: ImageGalleryController; onText: (text: string) => void }) {
  const i = g.interpretation;
  const missing = i?.clauses.filter(c => !c.excluded && c.matches === 0) ?? [];
  return <div className="image-empty" role="status">
    <h2>No images match</h2>
    {missing.length > 0 && <p>No image contains {missing.map(c => `“${c.text}”`).join(', ')} in the searched fields.</p>}
    {i?.suggestions.map(s => <p key={s.term}>Did you mean {s.candidates.map((c, n) => <span key={c}>{n ? ', ' : ''}<button className="link" onClick={() => onText((g.filters.text ?? '').replace(s.term, c))}>{c}</button></span>)}?</p>)}
    {!!i?.text_only_matches && <p>{i.text_only_matches.toLocaleString()} {i.text_only_matches === 1 ? 'image matches' : 'images match'} your words but not the other filters.</p>}
    {!!i?.ignored.length && <p>Ignored {i.ignored.map(t => `“${t}”`).join(', ')} (no letters or numbers).</p>}
    <p>Try fewer words, a prefix such as <code>beach*</code>, or remove a filter.</p>
    <button onClick={() => { onText(''); g.resetFilters(); }}>Clear search and filters</button>
  </div>;
}
