import { useEffect, useState } from 'react';
import { editImageGeneration, imageGeneration } from '../../api';
import type { ClaimSource, GenerationField, GenerationFieldView, GenerationPatch, GenerationView } from '../../types';
import { errorCode } from './annotationText';

/** Field labels; prompts are multi-line, the rest single-line. */
export const FIELD_LABELS: Record<GenerationField, string> = {
  prompt: 'Prompt', negative_prompt: 'Negative prompt', generator: 'Generator', model: 'Model', seed: 'Seed', sampler: 'Sampler',
  created: 'Created', settings: 'Settings', references: 'References', digital_source_type: 'IPTC source type', ai_generated: 'AI-generated',
};
const EDITABLE: GenerationField[] = ['prompt', 'negative_prompt', 'generator', 'model', 'seed', 'sampler', 'created', 'ai_generated'];
const MULTILINE: GenerationField[] = ['prompt', 'negative_prompt'];
const FORMATS: Record<ClaimSource['format'], string> = {
  a1111_parameters: 'A1111-style parameters', comfyui_prompt: 'ComfyUI prompt graph', invokeai_metadata: 'InvokeAI metadata',
  novelai_comment: 'NovelAI metadata', xmp_digital_source: 'IPTC digital source type', software_tag: 'Software tag', exif_datetime: 'EXIF date',
};
const CONTAINERS: Record<ClaimSource['container'], string> = {
  png_text: 'PNG text', png_compressed_text: 'PNG compressed text', png_international_text: 'PNG international text', png_exif: 'PNG EXIF',
  jpeg_exif: 'JPEG EXIF', jpeg_xmp: 'JPEG XMP', webp_exif: 'WebP EXIF', webp_xmp: 'WebP XMP', sidecar: 'sidecar file',
};
export const sourceLabel = (s: ClaimSource) => `${FORMATS[s.format]} · ${CONTAINERS[s.container]} “${s.key}”`;
const messages: Record<string, string> = {
  CONFLICT: 'Changed elsewhere. The latest provenance is shown; review it and try again.',
  SOURCE_UNAVAILABLE: 'The original or its source is offline, so provenance cannot be edited now.',
  PERMISSION_DENIED: 'This image is outside its approved source scope.',
  NOT_FOUND: 'This image is no longer in the library.',
  LIMIT_EXCEEDED: 'Too long: prompts are up to 64 KiB, other fields up to 1 KiB.',
  INVALID_ARGUMENT: 'That change is not valid.',
};
const failure = (e: unknown) => messages[errorCode(e)] ?? `Provenance change failed · ${errorCode(e).replaceAll('_', ' ').toLowerCase()}`;

function origin(f: GenerationFieldView) {
  if (f.origin === 'user') return 'Your entry';
  if (f.origin === 'imported' && f.source) return `Claimed by ${sourceLabel(f.source)}`;
  if (f.origin === 'inherited') return `Inherited from the parent image${f.source ? ` · ${sourceLabel(f.source)}` : ''}`;
  return f.redacted ? 'Removed when this library was exported' : 'Unknown';
}
function status(v: GenerationView) {
  const e = v.embedded;
  if (!e) return 'This version has not been read for generation metadata yet. Check files reads it.';
  const read = { none: 'No supported generation metadata in this file.', found: 'Generation metadata found in this file.', malformed: 'This file has generation metadata that could not be read.', limit_exceeded: 'This file’s metadata exceeded a reader limit and was not read.' }[e.status];
  const issues = e.issues.length ? ` ${e.issues.length} part${e.issues.length === 1 ? '' : 's'} could not be read.` : '';
  return read + issues;
}

/**
 * SS-055 provenance for one image: every value shows where it came from.
 * Imported values are claims written by other tools, never verified history;
 * all text renders as plain text and no reference or URL is opened.
 */
export function GenerationProvenance({ id, version, onChanged }: { id: string; version: string | null; onChanged?: () => void }) {
  const [view, setView] = useState<GenerationView | null>(null), [error, setError] = useState(''), [status_, setStatus] = useState('');
  const [draft, setDraft] = useState<Partial<Record<GenerationField, string>> | null>(null), [busy, setBusy] = useState(false), [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let live = true;
    imageGeneration(id).then(v => { if (live && v?.asset_id === id) setView(v); }).catch(e => { if (live) setError(failure(e)); });
    return () => { live = false; };
  }, [id, version, attempt]);
  const v = view?.asset_id === id ? view : null;
  if (!v) return error ? <p className="error" role="alert">{error}</p> : null;
  const apply = async (patch: GenerationPatch, label: string) => {
    setBusy(true); setError(''); setStatus('');
    try {
      const next = await editImageGeneration(id, v.revision, patch);
      setView(next); setDraft(null); setStatus(next.revision === v.revision ? 'No change.' : `${label}.`); onChanged?.();
    } catch (e) {
      setError(failure(e));
      if (errorCode(e) === 'CONFLICT') setAttempt(n => n + 1);
    } finally { setBusy(false); }
  };
  const save = () => {
    if (!draft) return;
    const set: GenerationPatch['set'] = {};
    for (const f of EDITABLE) {
      if (!(f in draft)) continue;
      const current = v.fields.find(x => x.field === f);
      const text = draft[f] ?? '';
      // An empty box removes your value (imported claims show again); a value is stored as typed.
      const next = text === '' ? null : text;
      if (current?.origin === 'user' ? current.value !== next : next !== null) set[f] = next;
    }
    void apply({ set }, 'Saved provenance');
  };
  const shown = v.fields.filter(f => f.value !== null || f.redacted || EDITABLE.includes(f.field));
  const extra = v.claims.flatMap(c => [
    ...Object.entries(c.fields.settings).map(([k, value]) => ({ key: `${sourceLabel(c.source)}:${k}`, label: k, value, source: c.source })),
    ...c.fields.references.map(r => ({ key: `${sourceLabel(c.source)}:${r.kind}:${r.value}`, label: `Reference (${r.kind.replace('_', ' ')}${r.private ? ', private' : ''})`, value: r.value, source: c.source })),
  ]);
  const unread = [...(v.embedded?.uninterpreted ?? []), ...(v.sidecar?.extraction.uninterpreted ?? [])];
  return <section className="image-generation" aria-labelledby={`image-generation-${id}`}>
    <h3 id={`image-generation-${id}`}>Generation provenance</h3>
    <p className="muted">Imported values are claims written by other tools, not verified history. Nothing is inferred from pixels.</p>
    <p role="status" className="muted">{status(v)}</p>
    {v.sidecar && <p className="muted">Sidecar {v.sidecar.relative_path}: {v.sidecar.present ? `version ${v.sidecar.revision}` : `removed (version ${v.sidecar.revision}); its values no longer apply`}.</p>}
    {draft ? <form className="image-generation-form" onSubmit={e => { e.preventDefault(); save(); }}>
      {EDITABLE.map(f => {
        const fid = `generation-${f}-${id}`;
        const value = draft[f] ?? '';
        return <div key={f} className="image-generation-input">
          <label htmlFor={fid}>{FIELD_LABELS[f]}</label>
          {f === 'ai_generated'
            ? <select id={fid} value={value} onChange={e => setDraft({ ...draft, [f]: e.target.value })}>
              <option value="">Not stated by you</option><option value="yes">Yes</option><option value="no">No</option>
            </select>
            : MULTILINE.includes(f)
              ? <textarea id={fid} rows={3} value={value} onChange={e => setDraft({ ...draft, [f]: e.target.value })} />
              : <input id={fid} value={value} onChange={e => setDraft({ ...draft, [f]: e.target.value })} />}
        </div>;
      })}
      <p className="muted">Leave a field empty to use the imported claim, if any.</p>
      <div className="image-notes-actions">
        <button type="submit" disabled={busy}>Save provenance</button>
        <button type="button" disabled={busy} onClick={() => setDraft(null)}>Cancel</button>
      </div>
    </form> : <dl className="image-facts image-generation-fields">
      {shown.map(f => <div key={f.field} className="image-generation-field">
        <dt>{FIELD_LABELS[f.field]}</dt>
        <dd>
          <span className={MULTILINE.includes(f.field) ? 'image-generation-text' : undefined}>{f.value === null ? '—' : f.value === '' ? '(recorded as empty)' : f.value}</span>
          <small className="muted">{origin(f)}{f.uncertain ? ' · time zone unknown' : ''}{f.conflicting ? ' · another claim records a different value' : ''}</small>
        </dd>
      </div>)}
    </dl>}
    {(extra.length > 0 || unread.length > 0) && <details className="image-generation-details">
      <summary>Imported settings and references ({extra.length + unread.length})</summary>
      <ul>
        {extra.map(x => <li key={x.key}><span>{x.label}: <span className="path-text">{x.value}</span></span> <small className="muted">{sourceLabel(x.source)}</small></li>)}
        {unread.map(u => <li key={`${u.container}:${u.key}`}><span>{u.key}: {u.bytes.toLocaleString()} bytes, kept unread and never run</span></li>)}
      </ul>
    </details>}
    {!draft && <div className="image-notes-actions">
      <button disabled={busy || !v.editable} onClick={() => setDraft(Object.fromEntries(EDITABLE.map(f => {
        const x = v.fields.find(y => y.field === f);
        return [f, x?.origin === 'user' ? x.value ?? '' : ''];
      })))}>Edit provenance</button>
      {v.claims.length > 0 && (v.confirmed
        ? <button disabled={busy || !v.editable} onClick={() => void apply({ confirm: false }, 'Review withdrawn')}>Withdraw review</button>
        : <button disabled={busy || !v.editable} onClick={() => void apply({ confirm: true }, 'Marked imported values as reviewed')}>Mark imported values reviewed</button>)}
    </div>}
    {v.confirmed && <p className="muted">You reviewed these imported values.</p>}
    {v.confirmation_outdated && <p className="muted">You reviewed an earlier version of these claims; the file or sidecar has changed since.</p>}
    {!v.editable && <p className="image-inspector-notice">Read only while the original is unavailable.</p>}
    <div aria-live="polite">{status_ && <span>{status_}</span>}</div>
    {error && <p className="error" role="alert">{error}</p>}
  </section>;
}
