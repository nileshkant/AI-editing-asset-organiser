import { useEffect, useState } from 'react';
import { cancelImagePreview, imagePreview } from '../../api';
import type { ImagePreviewMeta, ImagePreviewTier } from '../../types';

type State = { status: 'loading' } | { status: 'ready'; url: string; meta: ImagePreviewMeta } | { status: 'error'; code: string };
const messages: Record<string, string> = {
  STALE_VERSION: 'Preview out of date · the image changed',
  NOT_READY: 'Preview available after analysis',
  ORIGINAL_UNAVAILABLE: 'Original unavailable · no cached preview',
  NOT_FOUND: 'Image no longer in an authorized source',
  UNSUPPORTED: 'Unsupported image',
  LIMIT_EXCEEDED: 'Image exceeds preview limits',
  WORKER_UNAVAILABLE: 'Previews unavailable in this build',
};
const retryable = new Set(['BUSY', 'TIMED_OUT', 'SOURCE_CHANGED', 'STALE_VERSION', 'WORKER_FAILED', 'INTERNAL']);
const code = (e: unknown) => String(e instanceof Error ? e.message : e).replace(/^Error:\s*/, '');

type Props = {
  assetId: string; version: string; tier: ImagePreviewTier; label: string;
  /** `frame`: fixed 4rem box with caption; `fill`: fills its parent (grid, viewer). */
  variant?: 'frame' | 'fill'; tile?: [number, number]; onStale?: () => void;
  /** Wait this long while mounted before requesting, so fast scrolling issues no work. */
  delayMs?: number;
};
/**
 * Bounded preview by asset ID and content version. Obsolete requests are cancelled
 * and late replies ignored; owned blob URLs are revoked when the view changes.
 */
export function ImagePreview({ assetId, version, tier, label, variant = 'frame', tile, onStale, delayMs = 0 }: Props) {
  const [state, setState] = useState<State>({ status: 'loading' });
  const [attempt, setAttempt] = useState(0);
  const tileKey = tile ? `${tile[0]},${tile[1]}` : '';
  useEffect(() => {
    const requestId = crypto.randomUUID();
    let live = true, settled = false, started = false, url: string | null = null;
    setState({ status: 'loading' });
    const start = () => { started = true; imagePreview(requestId, assetId, version, tier, tile ?? null).then(({ meta, blob }) => {
      settled = true;
      if (!live) return;
      url = URL.createObjectURL(blob);
      setState({ status: 'ready', url, meta });
    }).catch(e => {
      settled = true;
      if (!live) return;
      const c = code(e);
      setState({ status: 'error', code: c });
      if (c === 'STALE_VERSION') onStale?.();
    }); };
    const timer = delayMs > 0 ? setTimeout(start, delayMs) : (start(), undefined);
    return () => {
      live = false;
      clearTimeout(timer);
      if (started && !settled) void cancelImagePreview(requestId).catch(() => {});
      if (url) URL.revokeObjectURL(url);
    };
  }, [assetId, version, tier, tileKey, attempt]); // eslint-disable-line react-hooks/exhaustive-deps
  const fill = variant === 'fill';
  if (state.status === 'loading') return <div className={fill ? 'image-preview-fill' : 'image-preview'} role="status" aria-label={`Loading preview of ${label}`}>
    {fill ? <span className="image-preview-pending" aria-hidden="true" /> : <div className="image-preview-frame" aria-hidden="true">…</div>}
  </div>;
  if (state.status === 'error') return <div className={fill ? 'image-preview-fill' : 'image-preview'} role="note" aria-label={`Preview of ${label} unavailable`}>
    {!fill && <div className="image-preview-frame" aria-hidden="true">—</div>}
    <small>{messages[state.code] || `Preview unavailable · ${state.code.replaceAll('_', ' ').toLowerCase()}`}</small>
    {retryable.has(state.code) && <button onClick={() => setAttempt(n => n + 1)} aria-label={`Retry preview of ${label}`}>Retry</button>}
  </div>;
  const { meta, url } = state;
  const alt = `${label}, ${meta.source_width} × ${meta.source_height}${meta.transparency ? ', transparent' : ''}`;
  // Checkerboard is a CSS background only; preview pixels keep their actual alpha.
  const img = <img src={url} width={meta.width} height={meta.height} className={meta.transparency ? 'checkerboard' : undefined} alt={alt} draggable={false} />;
  if (fill) return <div className="image-preview-fill">{img}{meta.offline_reference && <span className="image-preview-badge">Offline reference</span>}</div>;
  return <figure className="image-preview">
    <div className="image-preview-frame">{img}</div>
    {meta.offline_reference && <figcaption>Offline reference · original unavailable, export disabled</figcaption>}
  </figure>;
}
