import { useEffect, useState } from 'react';
import { Star, X } from 'lucide-react';
import { editImageAnnotations, undoImageAnnotationAction } from '../../api';
import type { AnnotationPatch, ImageDetails } from '../../types';
import { annotationError, errorCode, MAX_NOTE_BYTES, noteBytes, splitTags } from './annotationText';

type Props = {
  details: ImageDetails; name: string;
  /** Called after any change (including a conflict) so the inspector and grid reload. */
  onChanged: () => void;
};
/**
 * Creator annotations for one image. Every change is an explicit operation from
 * the revision last read; a concurrent change surfaces as a conflict, never a merge.
 * Notes and tags render as plain text.
 */
export function AnnotationEditor({ details: d, name, onChanged }: Props) {
  const [tagInput, setTagInput] = useState(''), [notes, setNotes] = useState(d.comment);
  const [status, setStatus] = useState(''), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const [undo, setUndo] = useState<{ id: string; label: string } | null>(null);
  const id = d.header.id, editable = d.header.availability === 'available' && d.source_available;
  // A reload after someone else's change replaces the draft only when it was not edited here.
  const [base, setBase] = useState(d.comment);
  useEffect(() => { if (notes === base) setNotes(d.comment); setBase(d.comment); }, [d.comment]); // eslint-disable-line react-hooks/exhaustive-deps
  const apply = async (patch: AnnotationPatch, label: string) => {
    setBusy(true); setError(''); setStatus('');
    try {
      const r = await editImageAnnotations(id, d.annotation_revision, patch);
      setStatus(r.action_id ? `${label}.` : 'No change.');
      setUndo(r.action_id ? { id: r.action_id, label } : null);
      if (patch.comment !== undefined) setBase(patch.comment);
      onChanged();
      return true;
    } catch (e) {
      setError(annotationError(e));
      if (errorCode(e) === 'CONFLICT') onChanged();
      return false;
    } finally { setBusy(false); }
  };
  const addTags = async () => {
    const tags = splitTags(tagInput);
    if (tags.length && await apply({ add_tags: tags }, `Added ${tags.join(', ')}`)) setTagInput('');
  };
  const revert = async () => {
    if (!undo) return;
    setBusy(true); setError('');
    try { const r = await undoImageAnnotationAction(undo.id); setStatus(r.conflicts ? 'Not undone: the image changed again after that edit.' : `Undid: ${undo.label}.`); setUndo(null); onChanged(); }
    catch (e) { setError(annotationError(e)); }
    finally { setBusy(false); }
  };
  const bytes = noteBytes(notes);
  return <section className="image-annotations" aria-labelledby="image-annotations-title">
    <h3 id="image-annotations-title">Your annotations</h3>
    {!editable && <p className="image-inspector-notice">Read only while the original is unavailable.</p>}
    <div className="image-annotation-row">
      <button className={`icon-button${d.favorite ? ' active' : ''}`} aria-pressed={d.favorite} disabled={busy || !editable}
        title={d.favorite ? 'Remove from favorites' : 'Mark as favorite'} onClick={() => void apply({ favorite: !d.favorite }, d.favorite ? 'Removed from favorites' : 'Marked favorite')}>
        <Star size={15} aria-hidden="true" fill={d.favorite ? 'currentColor' : 'none'} /><span className="sr-only">Favorite</span>
      </button>
      <fieldset className="image-rating" disabled={busy || !editable}>
        <legend>Rating</legend>
        {[null, 1, 2, 3, 4, 5].map(r => <label key={r ?? 0} title={r ? `${r} of 5` : 'No rating'}>
          <input type="radio" name={`image-rating-${id}`} checked={d.rating === r} aria-label={r ? `${r} of 5 stars` : 'No rating (unrated)'}
            onChange={() => void apply({ rating: r }, r ? `Rated ${r} of 5` : 'Cleared rating')} />
          <span aria-hidden="true">{r ?? 'None'}</span>
        </label>)}
      </fieldset>
    </div>
    <div className="image-tag-editor">
      <span className="image-tag-label" id={`image-tags-${id}`}>Tags</span>
      {d.tags.length ? <ul className="image-tags" aria-labelledby={`image-tags-${id}`}>{d.tags.map(t => <li key={t}>
        <span>{t}</span>
        <button className="tag-remove" aria-label={`Remove tag ${t}`} title={`Remove ${t}`} disabled={busy || !editable} onClick={() => void apply({ remove_tags: [t] }, `Removed ${t}`)}><X size={12} aria-hidden="true" /></button>
      </li>)}</ul> : <p className="muted">No tags</p>}
      <div className="image-tag-add">
        <label className="sr-only" htmlFor={`image-tag-input-${id}`}>Add tags to {name}</label>
        <input id={`image-tag-input-${id}`} value={tagInput} placeholder="Add tags, separated by commas" maxLength={2000} disabled={!editable}
          onChange={e => setTagInput(e.target.value)} onKeyDown={e => { if (e.key === 'Enter') { e.preventDefault(); void addTags(); } }} />
        <button disabled={busy || !editable || !splitTags(tagInput).length} onClick={() => void addTags()}>Add</button>
      </div>
    </div>
    <div className="image-notes">
      <label htmlFor={`image-notes-${id}`}>Notes</label>
      <textarea id={`image-notes-${id}`} value={notes} rows={3} disabled={!editable} aria-describedby={`image-notes-count-${id}`} onChange={e => setNotes(e.target.value)}
        // With an unsaved draft, Escape leaves the field instead of closing the inspector.
        onKeyDown={e => { if (e.key === 'Escape' && notes !== d.comment) { e.stopPropagation(); e.currentTarget.blur(); setStatus('Notes not saved yet.'); } }} />
      <div className="image-notes-actions">
        <small id={`image-notes-count-${id}`} className={bytes > MAX_NOTE_BYTES ? 'error' : 'muted'}>{bytes.toLocaleString()} / {MAX_NOTE_BYTES.toLocaleString()} bytes · plain text</small>
        <button disabled={busy || !editable || notes === d.comment || bytes > MAX_NOTE_BYTES} onClick={() => void apply({ comment: notes }, 'Saved notes')}>Save notes</button>
      </div>
    </div>
    <div className="image-annotation-status" aria-live="polite">
      {status && <span>{status}</span>}
      {undo && <button disabled={busy} onClick={() => void revert()}>Undo</button>}
    </div>
    {error && <p className="error" role="alert">{error}</p>}
  </section>;
}
