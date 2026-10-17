import { useEffect, useRef, useState } from 'react';
import { imageImportItems, importedImages } from '../../api';
import type { ImageImportItemPage, ImageImportJob, ImportedImagePage } from '../../types';
import type { ImageImportsController } from '../../hooks/useImageImports';
import { importTerminal } from '../../hooks/useImageImports';

function ItemDetails({ job }: { job: ImageImportJob }) {
  const [page, setPage] = useState<ImageImportItemPage | null>(null), [images, setImages] = useState<ImportedImagePage['items']>([]), [error, setError] = useState(''), [busy, setBusy] = useState(false);
  const turn = useRef(0);
  useEffect(() => {
    const current = ++turn.current; setBusy(true); setError('');
    void imageImportItems(job.id).then(value => { if (turn.current === current) setPage(value); }).catch(e => { if (turn.current === current) setError(String(e)); }).finally(() => { if (turn.current === current) setBusy(false); });
    void importedImages(job.source_id).then(value => { if (turn.current === current && value) setImages(value.items); }).catch(() => {});
    return () => { ++turn.current; };
  }, [job.id, job.sequence, job.source_id]);
  return <div>
    {error && <p role="alert">{error}</p>}
    <ul className="image-import-items">{page?.items.map(item => {
      const image = images.find(i => i.header.id === item.asset_id);
      return <li key={item.relative_path}><strong className="path-text">{item.relative_path}</strong> · {item.state}
        {image?.width && image?.height ? ` · ${image.width} × ${image.height}` : ''}
        {item.error_code && <span> · {item.error_code.replaceAll('_', ' ').toLowerCase()}</span>}
        {item.warnings.map(w => <small key={w}> · {w.replaceAll('_', ' ')}</small>)}
      </li>;
    })}</ul>
    {busy && <p role="status">Loading files…</p>}
    {page?.next_after && <button disabled={busy} onClick={() => {
      const current = turn.current; setBusy(true);
      void imageImportItems(job.id, page.next_after).then(next => { if (turn.current === current) setPage(old => ({ items: [...(old?.items || []), ...next.items], next_after: next.next_after })); }).catch(e => { if (turn.current === current) setError(String(e)); }).finally(() => { if (turn.current === current) setBusy(false); });
    }}>More files</button>}
  </div>;
}
function ImportJob({ job, controller }: { job: ImageImportJob; controller: ImageImportsController }) {
  const [expanded, setExpanded] = useState(false), [busy, setBusy] = useState(false);
  return <article className="job-row" aria-label={`${job.mode} import ${job.id}`}>
    <div className="job-heading"><strong>{job.mode === 'mixed' ? 'Images and audio' : 'Images'}</strong><span className={job.state === 'failed' ? 'import-error' : undefined}>{job.state}{!importTerminal(job) ? ` · ${job.phase}` : ''}</span></div>
    <progress aria-label="Media import progress" value={job.completed} max={Math.max(1, job.total)} />
    <p>{job.completed} / {job.total} files · {job.ready} ready · {job.reused} reused · {job.failed} failed · {job.unsupported} unsupported</p>
    <small className="path-text">{job.current}</small>
    {job.error_code && <p role={job.state === 'failed' ? 'alert' : 'status'}>{job.error_code.replaceAll('_', ' ').toLowerCase()}</p>}
    {job.history_truncated && <p>Older file statuses have expired. Library assets and your annotations are retained.</p>}
    <div className="image-import-actions">
      {!importTerminal(job) && <button disabled={busy} onClick={() => { setBusy(true); void controller.action(job, false).finally(() => setBusy(false)); }}>Cancel media import</button>}
      {importTerminal(job) && controller.enabled && <button disabled={busy} onClick={() => { setBusy(true); void controller.action(job, true).finally(() => setBusy(false)); }}>Retry media import</button>}
      <button aria-expanded={expanded} onClick={() => setExpanded(v => !v)}>{expanded ? 'Hide files' : 'Browse files'}</button>
    </div>
    {expanded && <ItemDetails job={job} />}
  </article>;
}
function Preview({ controller: c }: { controller: ImageImportsController }) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const opener = c.returnFocus.current || document.activeElement as HTMLElement | null, element = dialog.current;
    if (element?.showModal) element.showModal(); else element?.setAttribute('open', '');
    element?.querySelector('button')?.focus();
    return () => { element?.close?.(); opener?.focus(); };
  }, []);
  return <dialog ref={dialog} className="modal image-import-dialog" aria-labelledby="media-import-title" aria-modal="true"
    onCancel={event => { event.preventDefault(); c.close(); }} onKeyDown={event => {
      if (event.key === 'Escape') { event.preventDefault(); c.close(); }
      if (event.key === 'Tab') { const controls = event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'); const first = controls[0], last = controls[controls.length - 1]; if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); } else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); } }
    }}>
    <h2 id="media-import-title">Review media import</h2>
    <p>Files stay in their selected locations. Originals are never modified. No uploads or model downloads start.</p>
    {c.preparing && <p role="status">Checking selected files and folders…</p>}
    {c.preview && <>
      <p>Counts are estimates; damaged files may fail during import.</p>
      <p>{c.preview.images} images · {c.preview.audio} audio · {c.preview.unsupported} unsupported · {c.preview.inaccessible} inaccessible</p>
      {!c.preview.complete && <p role="status">Some locations could not be inspected. You can import accessible files; missing files will not be inferred from an incomplete scan.</p>}
      <ul>{c.preview.scopes.map((scope, i) => <li key={i}><strong>{scope.recursive ? 'Folder, including subfolders' : 'Selected files only'}</strong><p className="path-text">{scope.root}</p>{scope.files.map(file => <p key={file} className="path-text">{file}</p>)}</li>)}</ul>
      {c.preview.issues.map((issue, i) => <p key={i} className="path-text">{issue.path} · {issue.code.replaceAll('_', ' ').toLowerCase()}</p>)}
    </>}
    {c.previewError && <p role="alert">{c.previewError}</p>}
    <div className="modal-actions"><button disabled={c.confirming} onClick={c.close}>Cancel</button><button className="primary" disabled={!c.preview || c.preparing || c.confirming || c.preview.images + c.preview.audio === 0 || c.preview.expires_at * 1000 <= Date.now()} onClick={() => void c.confirm()}>{c.confirming ? 'Starting import…' : 'Confirm import'}</button></div>
  </dialog>;
}
export function ImageImports({ controller: c }: { controller: ImageImportsController }) {
  return <>
    {c.enabled && <section className="image-import-controls" aria-label="Media import">
      <h2>Import images and audio</h2><label>Include <select value={c.mode} onChange={event => c.setMode(event.target.value as 'images' | 'mixed')} disabled={c.preparing || c.confirming}><option value="mixed">Images and audio</option><option value="images">Images only</option></select></label>
      <div className="image-import-actions"><button disabled={c.preparing || c.confirming} onClick={() => void c.choose(false)}>Choose media files</button><button disabled={c.preparing || c.confirming} onClick={() => void c.choose(true)}>Choose media folders</button></div>
    </section>}
    {c.jobs.map(job => <ImportJob key={job.id} job={job} controller={c} />)}
    {(c.preparing || c.preview || c.previewError) && <Preview controller={c} />}
  </>;
}
