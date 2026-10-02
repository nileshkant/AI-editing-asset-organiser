import { useState, useRef } from 'react';
import { chooseExportDestination, exportClip, cancelExport } from '../../api';
import type { Clip, ExportOptions, ExportResult } from '../../types';

export function ClipExport({ clip, onExported }: { clip: Clip; onExported?: () => void }) {
  const [open, setOpen] = useState(false);
  const [format, setFormat] = useState<ExportOptions['format']>('wav');
  const [rate, setRate] = useState('');
  const [fadeIn, setFadeIn] = useState(String(clip.recipe.fade_in_ms));
  const [fadeOut, setFadeOut] = useState(String(clip.recipe.fade_out_ms));
  const [busy, setBusy] = useState(false);
  const [destinationId, setDestinationId] = useState<string | null>(null);
  const [result, setResult] = useState<ExportResult | null>(null);
  const [error, setError] = useState('');
  const [copied, setCopied] = useState(false);
  const pending = useRef(false);

  async function start() {
    if (pending.current) return;
    const fades = [Number(fadeIn), Number(fadeOut)];
    if (!fadeIn.trim() || !fadeOut.trim() || fades.some(v => !Number.isSafeInteger(v) || v < 0 || v > 4294967295)) {
      setError('Fades must be non-negative whole milliseconds.'); return;
    }
    pending.current = true;
    setBusy(true); setError(''); setResult(null); setCopied(false);
    try {
      const grant = await chooseExportDestination(format);
      if (!grant) return;
      setDestinationId(grant.id);
      const exported = await exportClip(grant.id, clip.id, clip.revision, {
        format, sample_rate: rate ? Number(rate) : null,
        fade_in_ms: fades[0], fade_out_ms: fades[1],
      });
      setResult(exported);
      if (exported.sound_id) onExported?.();
    } catch (e) { setError(String(e)); }
    finally { pending.current = false; setBusy(false); setDestinationId(null); }
  }

  async function copyPath() {
    try {
      await navigator.clipboard.writeText(result!.path);
      setCopied(true);
    } catch (e) { setError(`Could not copy path: ${String(e)}`); }
  }

  return <div className="clip-export">
    <button type="button" className="compact-button" disabled={clip.is_stale || busy}
      aria-expanded={open} onClick={() => setOpen(!open)}>Export clip {clip.name}</button>
    {open && <fieldset disabled={busy}>
      <legend>Export {clip.name}</legend>
      <label>Format <select aria-label="Export format" value={format} onChange={e => setFormat(e.target.value as ExportOptions['format'])}>
        <option value="wav">WAV (24-bit PCM)</option><option value="flac">FLAC (lossless)</option>
      </select></label>
      <label>Sample rate <select aria-label="Export sample rate" value={rate} onChange={e => setRate(e.target.value)}>
        <option value="">Source rate</option>{[44100, 48000, 96000].map(r => <option key={r} value={r}>{r} Hz</option>)}
      </select></label>
      <label>Fade in (ms) <input aria-label="Export fade in milliseconds" type="number" min="0" step="1" value={fadeIn} onChange={e => setFadeIn(e.target.value)} /></label>
      <label>Fade out (ms) <input aria-label="Export fade out milliseconds" type="number" min="0" step="1" value={fadeOut} onChange={e => setFadeOut(e.target.value)} /></label>
      <button type="button" onClick={() => void start()}>Choose destination and export</button>
    </fieldset>}
    {busy && <div role="status">{destinationId ? 'Rendering and verifying export…' : 'Choosing destination…'}</div>}
    {busy && destinationId && <button type="button" onClick={() => {
      void cancelExport(destinationId).catch(e => setError(String(e)));
    }}>Cancel export</button>}
    {error && <p role="alert">{error}</p>}
    {result && <div role="status">
      <p>Exported {result.frames} frames.</p>
      <p className="path-text">{result.path}</p>
      {result.warning && <p role="alert">{result.warning}</p>}
      <button type="button" onClick={() => void copyPath()}>{copied ? 'Path copied' : 'Copy exported path'}</button>
      <p className="muted">Import this audio file in your editor. The adjacent .soundshelf.json manifest records timing and provenance; both files work after SoundShelf closes.</p>
    </div>}
  </div>;
}
