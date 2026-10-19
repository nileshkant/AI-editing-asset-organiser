import { useCallback, useEffect, useState } from 'react';
import { imageTags, previewImageTagRename, removeImageTagAlias, renameImageTag, undoImageAnnotationAction } from '../../api';
import type { BulkResult, RenamePreview, TagList } from '../../types';
import { ImageModal } from './ImageModal';
import { annotationError, resultSummary, tagIdentity } from './annotationText';

type Props = { sourceId: string | null; sourceName: string; onClose: () => void; onChanged: () => void };

/**
 * Image tag rename and aliases. Renames cover the gallery's current source scope
 * (all sources, or one); aliases are library-wide, so they need an all-sources rename.
 * Audio tags are never changed here.
 */
export function TagManagerDialog({ sourceId, sourceName, onClose, onChanged }: Props) {
  const [list, setList] = useState<TagList | null>(null), [from, setFrom] = useState(''), [to, setTo] = useState('');
  const [alias, setAlias] = useState(true), [preview, setPreview] = useState<RenamePreview | null>(null), [result, setResult] = useState<BulkResult | null>(null);
  const [error, setError] = useState(''), [status, setStatus] = useState(''), [busy, setBusy] = useState(false);
  const load = useCallback(() => imageTags(sourceId).then(setList).catch(e => setError(annotationError(e))), [sourceId]);
  useEffect(() => { void load(); }, [load]);
  const run = async (fn: () => Promise<void>) => { setBusy(true); setError(''); try { await fn(); } catch (e) { setError(annotationError(e)); } finally { setBusy(false); } };
  const choose = (tag: string) => { setFrom(tag); setTo(tag); setPreview(null); setResult(null); setStatus(''); requestAnimationFrame(() => document.getElementById('image-tag-rename-to')?.focus()); };
  const respell = tagIdentity(from) === tagIdentity(to);
  const keepAlias = alias && !sourceId && !respell;
  const scope = sourceId ? `in ${sourceName}` : 'in all sources';
  return <ImageModal title="Image tags" className="image-tags-dialog" onClose={onClose}>
    <p className="muted">Showing tags {scope}. Renaming changes image tags only; audio tags are unchanged.</p>
    {!list ? <p role="status">Loading tags…</p> : !list.tags.length ? <p>No image tags yet.</p>
      : <ul className="image-tag-list" aria-label={`Image tags ${scope}`}>{list.tags.map(t => <li key={t.tag}>
        <button aria-pressed={from === t.tag} onClick={() => choose(t.tag)}><span className="path-text">{t.tag}</span> <span className="muted">{t.count.toLocaleString()}</span></button>
      </li>)}</ul>}
    {list?.truncated && <p className="muted">Showing the 1,000 most used tags.</p>}
    {from && <fieldset className="image-tag-rename">
      <legend>Rename “{from}”</legend>
      <label htmlFor="image-tag-rename-to">New name</label>
      <input id="image-tag-rename-to" value={to} onChange={e => { setTo(e.target.value); setPreview(null); setResult(null); setStatus(''); }} />
      <label className="checkbox"><input type="checkbox" checked={keepAlias} disabled={!!sourceId || respell} onChange={e => setAlias(e.target.checked)} />
        Keep “{from}” as an alias, so adding it later adds the new name</label>
      {sourceId && <p className="muted">Aliases apply to the whole library; choose All sources to keep one.</p>}
      {preview && !result && <p role="status">
        {preview.images.toLocaleString()} {preview.images === 1 ? 'image' : 'images'} will change{preview.merged ? ` (${preview.merged.toLocaleString()} already have “${preview.to}” and will keep one)` : ''}.
        {preview.unavailable ? ` ${preview.unavailable.toLocaleString()} offline or missing ${preview.unavailable === 1 ? 'image keeps' : 'images keep'} “${preview.from}”.` : ''}
        {preview.audio_unchanged ? ` ${preview.audio_unchanged.toLocaleString()} audio ${preview.audio_unchanged === 1 ? 'sound uses' : 'sounds use'} this tag and ${preview.audio_unchanged === 1 ? 'is' : 'are'} not changed.` : ''}
      </p>}
      {result && <p role="status">{resultSummary(result, 'Renamed on')}{status ? ` · ${status}` : ''}</p>}
      <div className="modal-actions">
        {!result && <button disabled={busy || !to.trim() || to.trim() === from} onClick={() => void run(async () => setPreview(await previewImageTagRename(from, to, sourceId)))}>Preview rename</button>}
        {preview && !result && <button className="primary" disabled={busy} onClick={() => void run(async () => { setResult(await renameImageTag(from, to, sourceId, keepAlias)); onChanged(); await load(); })}>Rename</button>}
        {result?.action_id && !status && <button disabled={busy} onClick={() => void run(async () => {
          const r = await undoImageAnnotationAction(result.action_id!);
          setStatus(r.conflicts ? `undone for ${r.applied}; ${r.conflicts} changed again and kept their newer tags` : 'undone');
          onChanged(); await load();
        })}>Undo rename</button>}
      </div>
    </fieldset>}
    {!!list?.aliases.length && <section className="image-aliases" aria-labelledby="image-aliases-title">
      <h3 id="image-aliases-title">Aliases</h3>
      <ul>{list.aliases.map(a => <li key={a.alias}><span>“{a.alias}” → “{a.canonical}”</span>
        <button disabled={busy} aria-label={`Remove alias ${a.alias}`} onClick={() => void run(async () => { await removeImageTagAlias(a.alias); await load(); })}>Remove</button></li>)}</ul>
      <p className="muted">Removing an alias does not change tags already on images.</p>
    </section>}
    {error && <p className="error" role="alert">{error}</p>}
  </ImageModal>;
}
