import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { call, imageDetails } from '../../api';
import type { ImageDetails, ImageSummary } from '../../types';
import type { ImageProfile } from '../../contracts/imageV1';
import { fileName } from './ImageGrid';
import { ImageViewer, type PreviewBackground } from './ImageViewer';
import { AnnotationEditor } from './AnnotationEditor';
import { GenerationProvenance } from './GenerationProvenance';

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

type Props = {
  summary: ImageSummary; background: PreviewBackground; setBackground: (b: PreviewBackground) => void; onClose: () => void; onStale: () => void;
  /** Bumped by the gallery after bulk edits, renames or undo so details reload. */
  annotationsVersion: number; onAnnotated: () => void;
};
const analysisStates: Record<string, string> = {
  pending: 'Queued', running: 'Running', dispatching: 'Running', paused: 'Paused',
  failed: 'Failed', unknown_outcome: 'Outcome unknown', stale: 'Source changed', revoked: 'Cancelled',
  done: 'Complete', reused: 'Complete', complete: 'Complete', cancelled: 'Cancelled',
  queued: 'Queued', disabled: 'No analysis yet',
};

function VisualAnalysis({ details, refresh }: { details: ImageDetails; refresh: () => void }) {
  const analysis = details.analysis;
  const latest = details.evidence?.slice().reverse().find(e =>
    e.origin === 'model' && !e.stale && e.review_state !== 'rejected');
  const [acceptCharge, setAcceptCharge] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const unknown = analysis?.state === 'unknown_outcome';
  const partial = latest?.visual && (latest.visual.uncertain || !latest.caption || latest.visual.tags.length === 0);
  const state = analysisStates[analysis?.state ?? details.header.enrichment_state] ?? 'Unknown';
  const retry = async () => {
    if (!analysis) return;
    setBusy(true); setError('');
    try {
      await call('image_enrichment_retry', { jobId: analysis.job_id, assetId: details.header.id, acceptPossibleCharge: acceptCharge });
      refresh();
    } catch (e) { setError(String(e)); }
    finally { setBusy(false); setAcceptCharge(false); }
  };
  return <section className="image-annotations" aria-label="AI analysis">
    <h3>AI analysis</h3>
    <p role="status">{partial && state === 'Complete' ? 'Partial' : state}
      {analysis?.error_code && <> · {analysis.error_code.replaceAll('_', ' ').toLowerCase()}</>}
    </p>
    {latest && <>
      {latest.caption && <p className="path-text">{latest.caption}</p>}
      {latest.visual?.uncertain && <p className="muted">The model was unsure what the image shows.</p>}
      {!!latest.visual?.tags.length && <ul className="image-tags" aria-label="Visual suggestions">
        {latest.visual.tags.map(tag => <li key={`${tag.category}:${tag.label}`} className="path-text">{tag.category} · {tag.label}</li>)}
      </ul>}
      {latest.model && <details>
        <summary>Analysis provenance</summary>
        <dl className="image-facts">
          <dt>Provider</dt><dd className="path-text">{latest.model.provider}</dd>
          <dt>Model revision</dt><dd className="path-text">{latest.model.model} · {latest.model.revision}</dd>
          {latest.visual && <><dt>Provider settings revision</dt><dd>{latest.visual.provider_revision}</dd></>}
          <dt>Input coverage</dt><dd>{latest.coverage === 'full_image' ? 'Full image' : 'Resized preview'}</dd>
          <dt>Confidence</dt><dd>{latest.confidence_kind === 'unavailable' ? 'Unavailable' : latest.confidence_kind}</dd>
          <dt>Analyzed</dt><dd>{new Date(latest.created_at_ms).toLocaleString()}</dd>
          <dt>Image version</dt><dd className="path-text">{latest.content_hash}</dd>
          <dt>Preprocessor</dt><dd className="path-text">{latest.model.preprocessing}</dd>
          <dt>Output schema</dt><dd className="path-text">{latest.model.output_schema}</dd>
        </dl>
      </details>}
    </>}
    {analysis?.retryable && details.header.availability === 'available' && <>
      {unknown && <label><input type="checkbox" checked={acceptCharge} onChange={e => setAcceptCharge(e.target.checked)} />I accept a possible second charge</label>}
      <button disabled={busy || (unknown && !acceptCharge)} onClick={() => void retry()}>Retry AI analysis</button>
    </>}
    {error && <p role="alert">{error}</p>}
  </section>;
}

export function ImageInspector({ summary, background, setBackground, onClose, onStale, annotationsVersion, onAnnotated }: Props) {
  const [details, setDetails] = useState<ImageDetails | null>(null), [error, setError] = useState(''), [attempt, setAttempt] = useState(0);
  const id = summary.header.id;
  useEffect(() => {
    let live = true;
    setError('');
    imageDetails(id).then(d => { if (live && d?.header?.id === id) setDetails(d); }).catch(e => { if (live) { setDetails(null); setError(String(e)); } });
    return () => { live = false; };
  }, [id, summary.header.content_hash, attempt, annotationsVersion]);
  const d = details?.header.id === id && details.header.content_hash === summary.header.content_hash ? details : null;
  const header = d?.header ?? summary.header, p = d?.profile;
  const analysisState = d?.analysis?.state;
  useEffect(() => {
    if (!analysisState || !['pending', 'running', 'dispatching'].includes(analysisState)) return;
    const timer = window.setInterval(() => setAttempt(n => n + 1), 2000);
    return () => window.clearInterval(timer);
  }, [analysisState]);
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
    </dl>
    {d && <AnnotationEditor details={d} name={name} onChanged={() => { setAttempt(n => n + 1); onAnnotated(); }} />}
    {d && <VisualAnalysis key={`${id}:${header.content_hash}:${d.analysis?.job_id}`} details={d} refresh={() => setAttempt(n => n + 1)} />}
    {d && <GenerationProvenance id={id} version={header.content_hash} />}
  </aside>;
}
