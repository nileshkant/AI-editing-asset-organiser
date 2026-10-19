import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { imageDetails } from '../../api';
import type { ImageDetails, ImageSummary } from '../../types';
import type { ImageProfile } from '../../contracts/imageV1';
import { fileName } from './ImageGrid';
import { ImageViewer, type PreviewBackground } from './ImageViewer';

export const formatBytes = (n: number) => n < 1024 ? `${n} B` : n < 1024 ** 2 ? `${(n / 1024).toFixed(1)} KiB` : `${(n / 1024 ** 2).toFixed(1)} MiB`;
/** Channel support and actual transparent pixels are distinct facts (SS-042). */
export function transparencyLabel(p: ImageProfile) {
  if (!p.alpha_channel) return 'No alpha channel · fully opaque';
  return { none: 'Alpha channel · every pixel opaque', binary: 'Binary transparency · some pixels fully transparent', partial: 'Partial transparency · semi-transparent pixels' }[p.transparency];
}
const availability: Record<string, string> = {
  available: 'Original available',
  offline: 'Original offline · cached previews only, export disabled',
  missing: 'Original missing · cached previews only, export disabled',
  inaccessible: 'Original inaccessible · cached previews only, export disabled',
};
export function BackgroundChoice({ value, onChange }: { value: PreviewBackground; onChange: (b: PreviewBackground) => void }) {
  return <fieldset className="image-background-choice">
    <legend>Preview background</legend>
    {(['checkerboard', 'light', 'dark'] as const).map(b => <label key={b}><input type="radio" name="image-preview-background" value={b} checked={value === b} onChange={() => onChange(b)} />{b === 'checkerboard' ? 'Checkerboard' : b === 'light' ? 'Light' : 'Dark'}</label>)}
  </fieldset>;
}

type Props = { summary: ImageSummary; background: PreviewBackground; setBackground: (b: PreviewBackground) => void; onClose: () => void; onStale: () => void };
export function ImageInspector({ summary, background, setBackground, onClose, onStale }: Props) {
  const [details, setDetails] = useState<ImageDetails | null>(null), [error, setError] = useState(''), [attempt, setAttempt] = useState(0);
  const id = summary.header.id;
  useEffect(() => {
    let live = true;
    setError('');
    imageDetails(id).then(d => { if (live && d?.header?.id === id) setDetails(d); }).catch(e => { if (live) { setDetails(null); setError(String(e)); } });
    return () => { live = false; };
  }, [id, summary.header.content_hash, attempt]);
  const d = details?.header.id === id ? details : null, header = d?.header ?? summary.header, p = d?.profile;
  const name = fileName(header.relative_path);
  const ready = header.technical_state === 'ready' && !!header.content_hash && !!p;
  const stale = () => { setAttempt(n => n + 1); onStale(); };
  return <aside className="inspector image-inspector" tabIndex={-1} aria-label={`Image details: ${name}`}
    onKeyDown={e => { if (e.key === 'Escape') { e.preventDefault(); onClose(); } }}>
    <div className="inspector-heading"><span>IMAGE DETAILS</span>
      <button className="icon-button" aria-label="Close image details" title="Close (Escape)" onClick={onClose}><X size={15} aria-hidden="true" /></button>
    </div>
    <h2>{name}</h2>
    {error && <p role="alert">Details unavailable · {error.replaceAll('_', ' ').toLowerCase()}</p>}
    {header.availability !== 'available' && <p className="image-inspector-notice" role="status">{availability[header.availability]}</p>}
    {ready ? <ImageViewer key={`${id}:${header.content_hash}`} assetId={id} version={header.content_hash!} width={p.width} height={p.height} label={name} background={background} onStale={stale} originalAvailable={header.availability === 'available'} />
      : <p className="image-inspector-notice">{header.technical_state === 'failed' ? 'Technical analysis failed; no preview is available.' : d ? 'Preview available after technical analysis.' : 'Loading details…'}</p>}
    <BackgroundChoice value={background} onChange={setBackground} />
    <dl className="image-facts">
      {p && <>
        <dt>Dimensions</dt><dd>{p.width} × {p.height} px{p.orientation !== 1 ? ` · stored ${p.raw_width} × ${p.raw_height}, EXIF orientation ${p.orientation}` : ''}</dd>
        <dt>Format</dt><dd>{p.format.toUpperCase()} · {formatBytes(p.encoded_bytes)} · {p.bit_depth}-bit</dd>
        <dt>Color</dt><dd>{p.source_color_profile ? 'Embedded ICC profile, displayed in sRGB' : 'No embedded profile, displayed as sRGB'}</dd>
        <dt>Transparency</dt><dd>{transparencyLabel(p)}</dd>
      </>}
      <dt>File</dt><dd className="path-text">{header.relative_path}</dd>
      <dt>Source</dt><dd>{d ? `${d.source_name}${d.source_available ? '' : ' · offline'}` : '…'}</dd>
      <dt>Availability</dt><dd>{availability[header.availability]}</dd>
      <dt>Technical analysis</dt><dd>{header.technical_state}</dd>
      <dt>AI analysis</dt><dd>{header.enrichment_state === 'disabled' ? 'Not enabled' : header.enrichment_state}</dd>
      <dt>Tags</dt><dd>{d?.tags.length ? <ul className="image-tags">{d.tags.map(t => <li key={t}>{t}</li>)}</ul> : 'No tags'}</dd>
    </dl>
  </aside>;
}
