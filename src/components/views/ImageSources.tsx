import { useState } from 'react';
import * as api from '../../api';
import type { ImageMoveCandidate, ImageSourceStatus } from '../../types';
import type { ImageSourcesController } from '../../hooks/useImageSources';

const rootLabel: Record<ImageSourceStatus['root_state'], string> = {
  online: 'Available',
  unknown: 'Not checked yet',
  offline: 'Offline · reconnect the drive, then check again',
  inaccessible: 'Access denied · check this folder’s permissions',
  changed: 'A different folder is at this location · relink to verify it',
};
function watchText(s: ImageSourceStatus) {
  if (!s.watch) return s.scope === 'files' ? 'Off. Watching covers only the selected files, never their siblings.' : 'Off. Watching uses system notifications for this folder only.';
  if (!s.watch_state) return 'Starting…';
  if (s.watch_state.state === 'watching') return 'Watching. Changes are checked by size and date; verify bytes for a full check.';
  if (s.watch_state.state === 'paused') return `Paused · ${s.watch_state.error || 'source unavailable'}`;
  return `Unavailable · ${s.watch_state.error || 'system watcher failed'}. Use Refresh instead.`;
}
function Moves({ source, controller }: { source: ImageSourceStatus; controller: ImageSourcesController }) {
  const [candidates, setCandidates] = useState<ImageMoveCandidate[] | null>(null), [busy, setBusy] = useState(false);
  const load = () => controller.act(async () => setCandidates(await api.imageMoveCandidates(source.source_id)));
  return <details onToggle={event => { if ((event.currentTarget as HTMLDetailsElement).open && !candidates) void load(); }}>
    <summary>Possible moved files ({source.move_candidates})</summary>
    <p>These missing records have the same bytes as a file now in another place. Reattaching moves the missing record’s identity, tags and notes to that path and removes the duplicate record. Files on disk are not changed.</p>
    {!candidates && <p role="status">Loading…</p>}
    {candidates && !candidates.length && <p>No possible moves remain.</p>}
    <ul className="image-import-items">{candidates?.map(c => <li key={`${c.missing_id}:${c.present_id}`}>
      <span className="path-text">{c.missing_path}</span> (missing) → <span className="path-text">{c.present_path}</span>{c.same_file ? ' · same file' : ''}
      <button disabled={busy || !!source.active_job} aria-label={`Reattach ${c.missing_path} to ${c.present_path}`} onClick={() => {
        setBusy(true);
        void controller.act(() => api.reconcileImageMove(c)).then(load).finally(() => setBusy(false));
      }}>Reattach</button>
    </li>)}</ul>
  </details>;
}
function SourceCard({ source: s, controller, onRelink }: { source: ImageSourceStatus; controller: ImageSourcesController; onRelink: (id: string) => void }) {
  const [busy, setBusy] = useState(false);
  const run = (task: () => Promise<unknown>) => { setBusy(true); void controller.act(task).finally(() => setBusy(false)); };
  const working = busy || !!s.active_job;
  const reachable = s.root_state === 'online' || s.root_state === 'unknown';
  return <article className="job-row" aria-label={`Image source ${s.name}`}>
    <div className="job-heading"><strong>{s.name}</strong><span role="status" className={reachable ? undefined : 'import-error'}>{rootLabel[s.root_state]}</span></div>
    <code className="path-text">{s.root}</code>
    <p>{s.images} images · {s.missing} missing · {s.inaccessible} access denied · {s.failed} could not be read · {s.pending} pending</p>
    {s.active_job && <p role="status">A check is running; see its progress below.</p>}
    <label className="settings-check"><input type="checkbox" checked={s.watch} disabled={busy} onChange={event => run(() => api.setImageSourceWatch(s.source_id, event.target.checked))} />Watch for changes</label>
    <p className="muted">{watchText(s)}</p>
    <div className="image-import-actions">
      {s.scope === 'folder' && <button disabled={working || !reachable} title="Find new, changed and removed files" onClick={() => run(() => api.refreshImageSource(s.source_id, 'discover', false))}>Refresh</button>}
      <button disabled={working || !reachable} title="Check only files already in the library" onClick={() => run(() => api.refreshImageSource(s.source_id, 'listed', false))}>Check files</button>
      <button disabled={working || !reachable} title="Read every byte; catches changes that keep the same size and date" onClick={() => run(() => api.refreshImageSource(s.source_id, 'listed', true))}>Verify file bytes</button>
      <button disabled={busy} onClick={() => run(() => api.checkImageSource(s.source_id))}>Check availability</button>
      <button disabled={working} onClick={() => onRelink(s.source_id)}>Relink…</button>
    </div>
    {s.move_candidates > 0 && <Moves source={s} controller={controller} />}
  </article>;
}
export function ImageSources({ controller, onRelink }: { controller: ImageSourcesController; onRelink: (id: string) => void }) {
  if (!controller.sources.length) return null;
  return <section className="image-import-controls" aria-label="Image sources">
    <h2>Image sources</h2>
    <p>Refresh and watching never change original files. Missing files are only inferred from a complete scan of an available folder.</p>
    {controller.sources.map(s => <SourceCard key={s.source_id} source={s} controller={controller} onRelink={onRelink} />)}
  </section>;
}
