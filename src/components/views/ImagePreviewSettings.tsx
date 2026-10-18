import { useCallback, useEffect, useState } from 'react';
import { call } from '../../api';
import type { ImagePreviewCacheStatus } from '../../types';

const MiB = 1024 * 1024;
const sizes = [128, 256, 512, 1024, 2048, 4096, 8192].map(n => n * MiB);
const label = (bytes: number) => bytes >= 1024 * MiB ? `${(bytes / (1024 * MiB)).toFixed(bytes % (1024 * MiB) ? 1 : 0)} GiB` : bytes >= MiB ? `${Math.round(bytes / MiB)} MiB` : `${Math.ceil(bytes / 1024)} KiB`;
const valid = (s: ImagePreviewCacheStatus | null | undefined): s is ImagePreviewCacheStatus => !!s && typeof s.entries === 'number' && typeof s.quota_bytes === 'number' && !!s.counters;

/** Counts and controls only; no source paths, asset IDs or preview bytes are shown. */
export function ImagePreviewSettings({ onError }: { onError: (error: string) => void }) {
  const [status, setStatus] = useState<ImagePreviewCacheStatus | null>(null);
  const [busy, setBusy] = useState(false), [confirm, setConfirm] = useState(false), [message, setMessage] = useState('');
  const refresh = useCallback(() => call<ImagePreviewCacheStatus>('image_preview_cache_status').then(s => { if (valid(s)) setStatus(s); }), []);
  useEffect(() => { void refresh().catch(() => {}); }, [refresh]);
  const run = async (task: () => Promise<void>) => { if (busy) return; setBusy(true); setMessage(''); try { await task(); } catch (e) { onError(String(e)); } finally { setBusy(false); } };
  if (!status || (!status.enabled && status.entries === 0)) return null;
  const c = status.counters;
  return <div className="settings-card"><section aria-label="Image previews">
    <h2>Image previews</h2>
    <p className="muted">Generated thumbnails and inspection previews. Rebuilt on demand; originals, analysis and tags are never changed.</p>
    <label className="settings-control">Preview cache size <select aria-label="Preview cache size" disabled={busy} value={status.quota_bytes}
      onChange={e => { const quota = Number(e.target.value); void run(async () => { const next = await call<ImagePreviewCacheStatus>('set_image_preview_cache_quota', { quotaBytes: quota }); if (valid(next)) setStatus(next); setMessage(`Preview cache limited to ${label(quota)}. Least recently used previews are removed first.`); }); }}>
      {!sizes.includes(status.quota_bytes) && <option value={status.quota_bytes}>{label(status.quota_bytes)}</option>}
      {sizes.map(n => <option key={n} value={n}>{label(n)}</option>)}
    </select></label>
    <p>{status.entries} previews · {label(status.bytes)} of {label(status.quota_bytes)}{status.enabled ? ` · ${status.active} generating` : ' · generation unavailable in this build'}</p>
    <p className="muted">{c.hits} cache hits · {c.generated} generated · {c.evicted} evicted · {c.repaired} repaired · {c.write_failures} write failures · {c.offline_served} offline references</p>
    <div className="settings-actions">
      <button disabled={busy} onClick={() => void run(refresh)}>Refresh preview counts</button>
      <button disabled={busy} onClick={() => setConfirm(true)}>Clear image previews</button>
    </div>
    {confirm && <div role="group" aria-label="Confirm preview cache clear">
      <p>Clear generated image previews? Original images, technical analysis, annotations and tags stay intact. Previews in use finish first.</p>
      <button disabled={busy} onClick={() => void run(async () => { const n = await call<number>('clear_image_preview_cache', { confirmed: true }); setConfirm(false); setMessage(`Removed ${n} image previews. They rebuild on demand.`); await refresh(); })}>Confirm preview clear</button>
      <button disabled={busy} onClick={() => setConfirm(false)}>Cancel preview clear</button>
    </div>}
    {message && <p role="status">{message}</p>}
  </section></div>;
}
