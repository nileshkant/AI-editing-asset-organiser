import { memo, useEffect, useRef, useState } from 'react';
import { FolderOpen } from 'lucide-react';
import { call, cancelImageRelink, chooseImageRelink, confirmImageRelink } from '../../api';
import type { ImageRelinkPreview, Source } from '../../types';

interface RelinkModalProps {
  source: Source;
  /** SS-047: image/mixed sources relink through the native picker and a verified preview. */
  image?: boolean;
  onClose: () => void;
  onRelinked: () => void;
  onError: (e: string) => void;
}

export const RelinkModal = memo(function RelinkModal({ source, image = false, onClose, onRelinked, onError }: RelinkModalProps) {
  const [path, setPath] = useState(source.root);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [preview, setPreview] = useState<ImageRelinkPreview | null>(null);
  const [acknowledged, setAcknowledged] = useState(false);
  const pending = useRef(false);
  const dialog = useRef<HTMLDialogElement>(null);
  const plan = useRef<string | null>(null);
  plan.current = preview?.id ?? null;
  // An abandoned preview is discarded so its folder grant cannot be confirmed later.
  useEffect(() => () => { if (plan.current) void cancelImageRelink(plan.current).catch(() => {}); }, []);

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const element = dialog.current;
    if (element?.showModal) element.showModal();
    else element?.setAttribute('open', '');
    (element?.querySelector<HTMLElement>('[data-initial-focus]') ?? element?.querySelector('input'))?.focus();
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
      {image ? <form onSubmit={event => { event.preventDefault(); if (preview) void run(async () => {
        await confirmImageRelink(preview.id, acknowledged);
        plan.current = null;
        onRelinked();
      }); }}>
        <h2 id="relink-title">Relink {source.name}</h2>
        <p>Choose the folder that now holds this source’s files. Every cataloged file is then verified by its bytes; library IDs, tags and notes are kept and nothing on disk is changed.</p>
        {preview && <div role="status">
          <p className="path-text">{preview.root}</p>
          <p>{preview.present} of {preview.members} cataloged files found · {preview.absent} not found</p>
          <p>Sample check: {preview.matched} of {preview.sampled} identical · {preview.changed} changed · {preview.unreadable} unreadable</p>
          {!preview.complete && <p>The check stopped early, so these counts are partial.</p>}
        </div>}
        {preview?.acknowledgement_required && <label className="settings-check"><input type="checkbox" checked={acknowledged} disabled={busy} onChange={event => setAcknowledged(event.target.checked)} />Changed files will be reanalyzed (tags and notes are kept) and files not found will be marked missing</label>}
        {error && <p role="alert">{error}</p>}
        {busy && <p role="status">Checking folder…</p>}
        <div className="modal-actions">
          <button type="button" disabled={busy} onClick={onClose}>Cancel</button>
          <button type="button" data-initial-focus disabled={busy} onClick={() => void run(async () => {
            const next = await chooseImageRelink(source.id);
            if (!next) return;
            if (plan.current) void cancelImageRelink(plan.current).catch(() => {});
            setAcknowledged(false); setPreview(next);
          })}><FolderOpen size={16} aria-hidden="true" />{preview ? 'Choose another folder' : 'Choose folder'}</button>
          <button type="submit" className="primary" disabled={busy || !preview || (preview.acknowledgement_required && !acknowledged)}>Relink and verify</button>
        </div>
      </form> :
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
      </form>}
    </dialog>
  );
});
