import { useEffect, useRef, useState } from 'react';
import { imageImportItems, importedImages } from '../../api';
import type { ImageImportItemPage, ImageImportJob, ImportedImagePage, Source } from '../../types';
import type { ImageImportsController } from '../../hooks/useImageImports';
import { importTerminal } from '../../hooks/useImageImports';
import { ImagePreview } from '../image/ImagePreview';

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
function ImageMetadataBrowser({ sources, jobs, previews }: { sources: Source[]; jobs: ImageImportJob[]; previews: boolean }) {
  const [preferred, setPreferred] = useState(''), [page, setPage] = useState<ImportedImagePage | null>(null), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const epoch = useRef(0);
  const sourceId = sources.some(s => s.id === preferred) ? preferred : sources[0]?.id || '';
  const revision = jobs.filter(j => j.source_id === sourceId).map(j => `${j.id}:${j.sequence}`).join(',');
  useEffect(() => {
    const current = ++epoch.current; setPage(null); setError('');
    if (!sourceId) { setBusy(false); return; }
    setBusy(true);
    void importedImages(sourceId).then(result => { if (epoch.current === current) setPage(result); }).catch(e => { if (epoch.current === current) setError(String(e)); }).finally(() => { if (epoch.current === current) setBusy(false); });
    return () => { ++epoch.current; };
  }, [sourceId, revision]);
  return <section className="image-import-controls" aria-label="Imported image library">
    <h2>Imported images</h2><p>Library records remain available after older import statuses expire.</p>
    <label>Image source <select value={sourceId} disabled={!sources.length} onChange={event => setPreferred(event.target.value)}>{!sources.length && <option value="">No sources yet</option>}{sources.map(s => <option value={s.id} key={s.id}>{s.name}</option>)}</select></label>
    {error && <p role="alert">{error}</p>}
    <ul className="image-import-items">{page?.items.map(image => <li key={image.header.id} className={previews ? 'image-import-item-preview' : undefined}>
      {previews && image.header.technical_state === 'ready' && image.header.content_hash && <ImagePreview assetId={image.header.id} version={image.header.content_hash} tier="small" label={image.header.relative_path} />}
      <span><strong className="path-text">{image.header.relative_path}</strong> · {image.header.technical_state} · {image.header.availability}{image.width && image.height ? ` · ${image.width} × ${image.height}` : ''}</span></li>)}</ul>
    {page && !page.items.length && <p>No images in this source.</p>}
    {busy && <p role="status">Loading images…</p>}
    {page?.next_after && <button disabled={busy} onClick={() => {
      const current = epoch.current; setBusy(true);
      void importedImages(sourceId, page.next_after).then(next => { if (epoch.current === current) setPage(old => ({ items: [...(old?.items || []), ...next.items], next_after: next.next_after })); }).catch(e => { if (epoch.current === current) setError(String(e)); }).finally(() => { if (epoch.current === current) setBusy(false); });
    }}>More images</button>}
  </section>;
}
export function ImageImports({ controller: c, sources = [] }: { controller: ImageImportsController; sources?: Source[] }) {
  return <>
    {c.enabled && <section className="image-import-controls" aria-label="Media import">
      <h2>Import images and audio</h2><label>Include <select value={c.mode} onChange={event => c.setMode(event.target.value as 'images' | 'mixed')} disabled={c.preparing || c.confirming}><option value="mixed">Images and audio</option><option value="images">Images only</option></select></label>
      <div className="image-import-actions"><button disabled={c.preparing || c.confirming} onClick={() => void c.choose(false)}>Choose media files</button><button disabled={c.preparing || c.confirming} onClick={() => void c.choose(true)}>Choose media folders</button></div>
    </section>}
    {(c.enabled || c.metadataPresent) && <ImageMetadataBrowser sources={sources} jobs={c.jobs} previews={c.enabled} />}
    {c.jobs.map(job => <ImportJob key={job.id} job={job} controller={c} />)}
    {(c.preparing || c.preview || c.previewError) && <Preview controller={c} />}
  </>;
}
