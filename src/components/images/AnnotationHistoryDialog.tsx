import { useCallback, useEffect, useState } from 'react';
import { imageAnnotationHistory, undoImageAnnotationAction } from '../../api';
import type { AnnotationAction } from '../../types';
import { ImageModal } from './ImageModal';
import { annotationError, resultSummary } from './annotationText';

/** The last 100 annotation actions (kept across restarts on this computer). */
export function AnnotationHistoryDialog({ onClose, onChanged }: { onClose: () => void; onChanged: () => void }) {
  const [actions, setActions] = useState<AnnotationAction[] | null>(null);
  const [error, setError] = useState(''), [status, setStatus] = useState(''), [busy, setBusy] = useState(false);
  const load = useCallback(() => imageAnnotationHistory(100).then(setActions).catch(e => setError(annotationError(e))), []);
  useEffect(() => { void load(); }, [load]);
  const undo = async (a: AnnotationAction) => {
    setBusy(true); setError('');
    try { setStatus(`${a.label}: ${resultSummary(await undoImageAnnotationAction(a.id), 'Restored')}`); onChanged(); await load(); }
    catch (e) { setError(annotationError(e)); }
    finally { setBusy(false); }
  };
  return <ImageModal title="Annotation history" className="image-history-dialog" onClose={onClose}>
    <p className="muted">Undo restores an image only if it has not changed since; newer edits always win. Each action can be undone once.</p>
    {!actions ? <p role="status">Loading history…</p> : !actions.length ? <p>No annotation changes yet.</p>
      : <ol className="image-history">{actions.map(a => <li key={a.id}>
        <span><span className="path-text">{a.label}</span><small className="muted"> · {new Date(a.created_at * 1000).toLocaleString()}</small></span>
        {a.state === 'applied' ? <button disabled={busy} aria-label={`Undo ${a.label}`} onClick={() => void undo(a)}>Undo</button> : <small className="muted">Undone</small>}
      </li>)}</ol>}
    {status && <p role="status">{status}</p>}
    {error && <p className="error" role="alert">{error}</p>}
  </ImageModal>;
}
