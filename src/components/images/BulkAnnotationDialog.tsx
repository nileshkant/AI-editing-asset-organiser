import { useState } from 'react';
import { applyImageBulkAnnotations, previewImageBulkAnnotations, undoImageAnnotationAction } from '../../api';
import type { AnnotationPatch, BulkPreview, BulkResult } from '../../types';
import { ImageModal } from './ImageModal';
import { annotationError, MAX_BULK, resultSummary, splitTags } from './annotationText';

type Props = { ids: string[]; onClose: () => void; onChanged: () => void };
const outcomes: Record<string, string> = { source_unavailable: 'offline or missing', not_found: 'no longer in the library', permission_denied: 'outside its approved scope', limit_exceeded: 'would exceed 64 tags' };

/**
 * Bulk tags, favorite and rating for a selection. Preview shows counts and takes
 * revision tokens; applying skips images that changed since and reports each.
 */
export function BulkAnnotationDialog({ ids, onClose, onChanged }: Props) {
  const [add, setAdd] = useState(''), [remove, setRemove] = useState('');
  const [favorite, setFavorite] = useState(''), [rating, setRating] = useState('');
  const [preview, setPreview] = useState<BulkPreview | null>(null), [result, setResult] = useState<BulkResult | null>(null);
  const [error, setError] = useState(''), [status, setStatus] = useState(''), [busy, setBusy] = useState(false);
  const patch = (): AnnotationPatch => ({
    add_tags: splitTags(add), remove_tags: splitTags(remove),
    ...(favorite ? { favorite: favorite === 'mark' } : {}),
    ...(rating ? { rating: rating === 'clear' ? null : Number(rating) } : {}),
  });
  const p = patch(), empty = !p.add_tags!.length && !p.remove_tags!.length && p.favorite === undefined && p.rating === undefined;
  const edited = (fn: (v: string) => void) => (v: string) => { fn(v); setPreview(null); setResult(null); setStatus(''); };
  const run = async (fn: () => Promise<void>) => { setBusy(true); setError(''); try { await fn(); } catch (e) { setError(annotationError(e)); } finally { setBusy(false); } };
  const over = ids.length > MAX_BULK;
  return <ImageModal title={`Edit ${ids.length.toLocaleString()} selected ${ids.length === 1 ? 'image' : 'images'}`} className="image-bulk-dialog" onClose={onClose}>
    {over && <p className="error" role="alert">Bulk edits are limited to {MAX_BULK.toLocaleString()} images. Clear some of the selection.</p>}
    <p className="muted">Adds and removes only the tags you list; other tags, notes and AI suggestions stay as they are.</p>
    <label>Add tags <input data-autofocus value={add} onChange={e => edited(setAdd)(e.target.value)} placeholder="e.g. Brand A, Summer campaign" /></label>
    <label>Remove tags <input value={remove} onChange={e => edited(setRemove)(e.target.value)} placeholder="Comma separated" /></label>
    <div className="image-bulk-row">
      <label>Favorite <select value={favorite} onChange={e => edited(setFavorite)(e.target.value)}>
        <option value="">No change</option><option value="mark">Mark favorite</option><option value="unmark">Remove favorite</option>
      </select></label>
      <label>Rating <select value={rating} onChange={e => edited(setRating)(e.target.value)}>
        <option value="">No change</option><option value="clear">Clear rating</option>{[1, 2, 3, 4, 5].map(r => <option key={r} value={r}>{r} of 5</option>)}
      </select></label>
    </div>
    {preview && !result && <div className="image-bulk-preview" role="status">
      <p><strong>{preview.changes.toLocaleString()}</strong> {preview.changes === 1 ? 'image' : 'images'} will change{preview.unchanged ? `; ${preview.unchanged.toLocaleString()} already match` : ''}.</p>
      <ul>
        {preview.additions.map(a => <li key={`a-${a.tag}`}>Add “{a.tag}”: {a.count.toLocaleString()} already have it</li>)}
        {preview.removals.map(a => <li key={`r-${a.tag}`}>Remove “{a.tag}”: {a.count.toLocaleString()} currently have it</li>)}
      </ul>
      {preview.skipped.length > 0 && <p>{preview.skipped.length.toLocaleString()} skipped: {Object.entries(preview.skipped.reduce<Record<string, number>>((m, s) => ({ ...m, [s.outcome]: (m[s.outcome] ?? 0) + 1 }), {})).map(([o, n]) => `${n} ${outcomes[o] ?? o}`).join(', ')}.</p>}
      <p className="muted">One step: if storage fails, nothing changes. Images edited elsewhere after this preview are left alone and reported.</p>
    </div>}
    {result && <p role="status">{resultSummary(result)}{status ? ` · ${status}` : ''}</p>}
    {error && <p className="error" role="alert">{error}</p>}
    <div className="modal-actions">
      {!result && <button disabled={busy || empty || over} onClick={() => void run(async () => setPreview(await previewImageBulkAnnotations(ids, p)))}>Preview changes</button>}
      {preview && !result && <button className="primary" disabled={busy || !preview.changes}
        onClick={() => void run(async () => { setResult(await applyImageBulkAnnotations(preview.targets, p)); onChanged(); })}>
        Apply to {preview.changes.toLocaleString()} {preview.changes === 1 ? 'image' : 'images'}</button>}
      {result?.action_id && !status && <button disabled={busy} onClick={() => void run(async () => {
        const r = await undoImageAnnotationAction(result.action_id!);
        setStatus(r.conflicts ? `undone for ${r.applied}; ${r.conflicts} changed again and kept their newer annotations` : 'undone');
        onChanged();
      })}>Undo</button>}
    </div>
  </ImageModal>;
}
