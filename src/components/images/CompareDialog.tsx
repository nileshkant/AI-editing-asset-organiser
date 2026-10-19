import { useEffect, useRef, useState } from 'react';
import { imageDetails } from '../../api';
import type { ImageDetails, ImageSummary } from '../../types';
import { ImagePreview } from '../image/ImagePreview';
import { fileName } from './ImageGrid';
import { BackgroundChoice, transparencyLabel } from './ImageInspector';
import type { PreviewBackground } from './ImageViewer';

/** Bounded so at most four inspect previews (≤ 4 MiB each) are held at once. */
export const MAX_COMPARE = 4;

function Candidate({ summary, background }: { summary: ImageSummary; background: PreviewBackground }) {
  const [d, setD] = useState<ImageDetails | null>(null), [error, setError] = useState('');
  useEffect(() => { let live = true; imageDetails(summary.header.id).then(v => { if (live) setD(v); }).catch(e => { if (live) setError(String(e)); }); return () => { live = false; }; }, [summary.header.id]);
  const header = d?.header ?? summary.header, name = fileName(header.relative_path), ready = header.technical_state === 'ready' && !!header.content_hash;
  return <figure className="image-compare-item">
    <div className={`image-compare-preview bg-${background}`}>
      {ready ? <ImagePreview assetId={header.id} version={header.content_hash!} tier="inspect" label={name} variant="fill" /> : <small>{error ? 'Image unavailable' : 'No preview yet'}</small>}
    </div>
    <figcaption><strong className="path-text">{name}</strong>
      {d?.profile && <span>{d.profile.width} × {d.profile.height} · {d.profile.format.toUpperCase()} · {transparencyLabel(d.profile)}</span>}
      {header.availability !== 'available' && <span>Original {header.availability} · export disabled</span>}
    </figcaption>
  </figure>;
}
export function CompareDialog({ candidates, background, setBackground, onClose }: { candidates: ImageSummary[]; background: PreviewBackground; setBackground: (b: PreviewBackground) => void; onClose: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null, el = dialog.current;
    if (el?.showModal) el.showModal(); else el?.setAttribute('open', '');
    el?.querySelector<HTMLButtonElement>('button')?.focus();
    return () => { el?.close?.(); opener?.focus(); };
  }, []);
  return <dialog ref={dialog} className="modal image-compare-dialog" aria-labelledby="image-compare-title" aria-modal="true"
    onCancel={e => { e.preventDefault(); onClose(); }}
    onKeyDown={e => {
      if (e.key === 'Escape') { e.preventDefault(); onClose(); }
      if (e.key === 'Tab') {
        const controls = e.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled)');
        const first = controls[0], last = controls[controls.length - 1];
        if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
        else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
      }
    }}>
    <div className="image-compare-heading"><h2 id="image-compare-title">Compare {candidates.length} images</h2><button onClick={onClose}>Close comparison</button></div>
    <BackgroundChoice value={background} onChange={setBackground} />
    <div className="image-compare-grid" style={{ ['--compare-count' as string]: candidates.length }}>
      {candidates.slice(0, MAX_COMPARE).map(c => <Candidate key={c.header.id} summary={c} background={background} />)}
    </div>
  </dialog>;
}
