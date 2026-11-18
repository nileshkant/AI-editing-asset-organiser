import { useCallback, useEffect, useRef, useState } from 'react';
import { call } from '../../api';
import type { ModelPacksOverview, PackCapability, PackPhase, PackView } from '../../types';

const capabilityLabel: Record<PackCapability, string> = { image_description: 'Image descriptions', image_embedding: 'Search by meaning' };
const phaseLabel: Record<PackPhase, string> = {
  not_installed: 'Not installed', downloading: 'Downloading', verifying: 'Verifying files', verified: 'Downloaded and verified, not in use',
  active: 'Active', failed: 'Failed', incompatible: 'Not compatible with this computer',
};
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ['KiB', 'MiB', 'GiB', 'TiB'];
  let value = bytes / 1024, unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

function Pack({ pack, busy, run }: { pack: PackView; busy: boolean; run: (command: string, args?: Record<string, unknown>) => Promise<void> }) {
  const [removing, setRemoving] = useState(false);
  const ref = { id: pack.id, version: pack.version };
  const name = `${pack.display_name} ${pack.version}`;
  const running = pack.phase === 'downloading' || pack.phase === 'verifying';
  const noticeId = `notice-${pack.id}-${pack.version}`;
  return <article className="setting-row" aria-label={`Pack ${name}`}>
    <div className="folder-detail">
      <strong>{pack.display_name}</strong> <small className="muted">version {pack.version} · {capabilityLabel[pack.capability]}</small>
      <div>Status: <strong>{phaseLabel[pack.phase]}</strong>{pack.in_use > 0 && <small className="muted"> · used by {pack.in_use} running task{pack.in_use === 1 ? '' : 's'}</small>}</div>
      {pack.progress && <progress aria-label={`Download progress for ${name}`} max={pack.progress[1]} value={pack.progress[0]}>{formatBytes(pack.progress[0])} of {formatBytes(pack.progress[1])}</progress>}
      {pack.progress && <small className="muted">{formatBytes(pack.progress[0])} of {formatBytes(pack.progress[1])}</small>}
      {pack.detail && <p role="alert">{pack.detail}</p>}
      {pack.incompatible && <p>{pack.incompatible.detail}.</p>}
      {pack.update_available && <p>Version {pack.update_available} is available. Your current version keeps working until you switch.</p>}
      <small className="muted">
        Download {formatBytes(pack.download_bytes)}
        {pack.installed_bytes !== null && <> · on disk {formatBytes(pack.installed_bytes)}</>}
        {' '}· about {formatBytes(pack.min_ram_bytes)} of memory · runtime {pack.runtime}
        {pack.memory_unchecked && ' · this computer did not report its memory, so that was not checked'}
      </small>
      <details>
        <summary>License: {pack.license_spdx}</summary>
        <p id={noticeId}>{pack.notice}</p>
        {pack.notice_url && <small className="muted">{pack.notice_url}</small>}
      </details>
      {removing && <p role="alertdialog" aria-label={`Confirm removing ${name}`}>
        Remove {name}? Its downloaded files are deleted{pack.phase === 'active' ? ' and the pack stops being used' : ''}. Your tags and annotations are kept.{' '}
        <button disabled={busy} onClick={() => void run('model_pack_uninstall', { ...ref, confirmed: true }).then(() => setRemoving(false))}>Remove {name}</button>{' '}
        <button onClick={() => setRemoving(false)}>Keep it</button>
      </p>}
    </div>
    <div className="row-actions">
      {(pack.phase === 'not_installed' || pack.phase === 'failed') && !pack.incompatible && !pack.retired &&
        <button disabled={busy} onClick={() => void run('model_pack_install', ref)}>{pack.resumable_bytes > 0 ? `Resume download of ${name}` : pack.phase === 'failed' ? `Retry ${name}` : `Download ${name}`}</button>}
      {pack.phase === 'not_installed' && pack.resumable_bytes > 0 && <button disabled={busy} onClick={() => void run('model_pack_discard_partial', ref)}>Discard partial download of {name}</button>}
      {running && <button onClick={() => void call('model_pack_cancel_install')}>Cancel download of {name}</button>}
      {pack.phase === 'verified' && !pack.retired && <button disabled={busy} onClick={() => void run('model_pack_activate', ref)}>Activate {name}</button>}
      {pack.phase === 'active' && <button disabled={busy} onClick={() => void run('model_pack_deactivate', { capability: pack.capability })}>Deactivate {name}</button>}
      {(pack.phase === 'verified' || pack.phase === 'active' || (pack.phase === 'failed' && pack.installed_bytes !== null)) && !removing &&
        <button disabled={busy} onClick={() => setRemoving(true)}>Remove {name}</button>}
    </div>
  </article>;
}

export function ModelPackSettings({ onError }: { onError: (message: string) => void }) {
  const [overview, setOverview] = useState<ModelPacksOverview | null>(null);
  const [busy, setBusy] = useState(false);
  const latest = useRef(0);
  const refresh = useCallback(async () => {
    const ticket = ++latest.current;
    try {
      const next = await call<ModelPacksOverview>('model_packs');
      if (ticket === latest.current) setOverview(next);
    } catch (e) { onError(String(e)); }
  }, [onError]);
  useEffect(() => { void refresh(); }, [refresh]);
  const transferring = overview?.packs.some(p => p.phase === 'downloading' || p.phase === 'verifying') ?? false;
  useEffect(() => {
    if (!transferring) return;
    const timer = setInterval(() => void refresh(), 1000);
    return () => clearInterval(timer);
  }, [transferring, refresh]);
  const run = async (command: string, args?: Record<string, unknown>) => {
    // Installing takes minutes: show progress from polling while the call is pending.
    const install = command === 'model_pack_install';
    if (!install) setBusy(true);
    const poll = install ? setTimeout(() => void refresh(), 150) : undefined;
    try { await call(command, args); }
    catch (e) { const message = String(e); if (!/Cancelled/.test(message)) onError(message); }
    finally { if (poll) clearTimeout(poll); setBusy(false); await refresh(); }
  };
  if (!overview) return <section aria-label="Local model packs"><h2>Local model packs</h2><p className="muted">Loading…</p></section>;
  const { storage } = overview;
  return <section aria-label="Local model packs">
    <h2>Local model packs</h2>
    <p className="muted">Optional. CreativeShelf works fully without any model: importing, browsing, tags, search and AI clients connected through the MCP server do not need a pack. A pack is only downloaded when you press Download, comes from a fixed list the app trusts, and is checked against its recorded size and fingerprint before it can be used. Nothing from your catalog is sent when downloading.</p>
    {!overview.packs.length && <p>No local model packs are available yet. None has passed the quality evaluation, so nothing can be downloaded. Everything else keeps working.</p>}
    {overview.packs.length > 0 && !overview.runtime_available && <p role="status">This version cannot run local models yet. Packs can be downloaded and kept ready, but they are not used for analysis.</p>}
    {overview.pending_reindex.length > 0 && <p role="status">The active model for {overview.pending_reindex.map(n => capabilityLabel[n.capability]).join(', ')} changed. Existing search data from the previous model is kept separate and is not mixed with the new one; it will be rebuilt when analysis is available.</p>}
    {overview.packs.map(p => <Pack key={`${p.id}@${p.version}`} pack={p} busy={busy} run={run} />)}
    <p className="muted" aria-label="Pack storage">
      Packs on disk: {formatBytes(storage.installed_bytes)} · interrupted downloads: {formatBytes(storage.partial_bytes)}
      {storage.free_bytes !== null && <> · free space: {formatBytes(storage.free_bytes)}</>}
    </p>
  </section>;
}
