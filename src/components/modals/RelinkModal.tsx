import { memo, useEffect, useRef, useState } from 'react';
import { FolderOpen } from 'lucide-react';
import { call } from '../../api';
import type { Source } from '../../types';

interface RelinkModalProps {
  source: Source;
  onClose: () => void;
  onRelinked: () => void;
  onError: (e: string) => void;
}

export const RelinkModal = memo(function RelinkModal({ source, onClose, onRelinked, onError }: RelinkModalProps) {
  const [path, setPath] = useState(source.root);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const pending = useRef(false);
  const dialog = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const element = dialog.current;
    if (element?.showModal) element.showModal();
    else element?.setAttribute('open', '');
    element?.querySelector('input')?.focus();
    return () => { element?.close?.(); opener?.focus(); };
  }, []);

  const run = async (task: () => Promise<void>) => {
    if (pending.current) return;
    pending.current = true;
    setBusy(true);
    setError('');
    try { await task(); }
    catch (e) { setError(String(e)); onError(String(e)); }
    finally { pending.current = false; setBusy(false); }
  };

  return (
    <dialog ref={dialog} className="modal relink-dialog" aria-labelledby="relink-title" aria-modal="true"
      onCancel={event => { event.preventDefault(); if (!busy) onClose(); }}
      onKeyDown={event => {
        if (event.key === 'Escape') { event.preventDefault(); if (!busy) onClose(); }
        if (event.key === 'Tab') {
          const controls = event.currentTarget.querySelectorAll<HTMLElement>('input:not(:disabled), button:not(:disabled)');
          const first = controls[0], last = controls[controls.length - 1];
          if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
          else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
        }
      }}>
      <form onSubmit={event => { event.preventDefault(); void run(async () => {
        await call('relink_source', { id: source.id, path: path.trim() });
        onRelinked();
      }); }}>
        <h2 id="relink-title">Relink {source.name}</h2>
        <label>Folder path<input value={path} disabled={busy} onChange={event => setPath(event.target.value)} /></label>
        {error && <p role="alert">{error}</p>}
        {busy && <p role="status">Verifying folder…</p>}
        <div className="modal-actions">
          <button type="button" disabled={busy} onClick={onClose}>Cancel</button>
          <button type="button" disabled={busy} onClick={() => void run(async () => {
            const chosen = await call<string | null>('choose_folder');
            if (chosen) setPath(chosen);
          })}><FolderOpen size={16} aria-hidden="true" />Browse</button>
          <button type="submit" className="primary" disabled={busy || !path.trim()}>Verify &amp; relink</button>
        </div>
      </form>
    </dialog>
  );
});
