import { useEffect, useState } from 'react';
import { call } from '../../api';

type Preview = { token: string; preview: { schema: string; sources: { id: string; name: string }[]; sounds: number; legacy: boolean } };
type Report = { report: { sounds: number; sources: number; offline_sources: number }; warnings: string[] };
type Evidence = { sound_id: string; description: string; provenance: string; analyzed_seconds: number | null; measurement_scope: string };

export function CatalogData({ onError }: { onError: (message: string) => void }) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [roots, setRoots] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [evidence, setEvidence] = useState<Evidence[]>([]);
  const [showEvidence, setShowEvidence] = useState(false);
  useEffect(() => () => { if (preview) void call('catalog_cancel_preview', { token: preview.token }).catch(() => {}); }, [preview]);
  const action = async (task: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); setMessage('');
    try { await task(); } catch (e) { onError(String(e)); } finally { setBusy(false); }
  };
  const openImport = (legacy: boolean) => action(async () => {
    const next = await call<Preview | null>('catalog_preview', { legacy });
    if (next) { setPreview(next); setRoots({}); }
  });
  const exportCatalog = (format: 'json' | 'md') => action(async () => {
    const path = await call<string | null>('catalog_export', { format });
    if (path) setMessage(`Catalog exported to ${path}`);
  });
  const importCatalog = () => action(async () => {
    if (!preview) return;
    // Preview is consumed on submission, including failed validation.
    const token = preview.token;
    try {
      const result = await call<Report>('catalog_import', { token, confirmed: true });
      setMessage(`Imported ${result.report.sounds} sounds from ${result.report.sources} sources. ${result.report.offline_sources} sources need relinking. ${result.warnings.join(' ')}`);
    } finally { setPreview(null); }
  });
  return <div className="catalog-data">
    <h2>Data</h2>
    <p className="muted">Transfer your catalog, annotations, saved searches and clips. JSON restores metadata; Markdown provides a readable copy. Audio stays in your source folders.</p>
    <div className="header-actions">
      <button disabled={busy || !!preview} onClick={() => void exportCatalog('json')}>Export catalog JSON</button>
      <button disabled={busy || !!preview} onClick={() => void exportCatalog('md')}>Export catalog Markdown</button>
      <button disabled={busy || !!preview} onClick={() => void openImport(false)}>Import catalog</button>
      <button disabled={busy || !!preview} onClick={() => void openImport(true)}>Import legacy catalog</button>
    </div>
    {busy && <p role="status">Working on catalog…</p>}
    {message && <p role="status">{message}</p>}
    {preview && <section className="catalog-preview" aria-label="Catalog import preview">
      <h3>{preview.preview.legacy ? 'Review legacy import' : 'Review catalog import'}</h3>
      <p>{preview.preview.sounds.toLocaleString()} sounds · {preview.preview.sources.length} sources</p>
      <p>Choose a local folder for each source. Existing catalog entries are preserved; conflicting identities or overlapping sources stop the import.</p>
      {preview.preview.legacy
        ? <p>Map every source before importing. Legacy descriptions are filename inference. Old measurements cover only the first 20 seconds at 12 kHz mono. Full-file analysis will run in the background.</p>
        : <p>Unmapped sources are imported offline. Relink them in Sources when their folders are available. Import reuses stored analysis without reading audio.</p>}
      {preview.preview.sources.map(source => <div className="setting-row" key={source.id}>
        <div className="folder-detail"><strong>{source.name}</strong><small>{roots[source.id] || 'Not mapped'}</small></div>
        <button disabled={busy} onClick={() => void action(async () => {
          const root = await call<string | null>('catalog_map_root', { token: preview.token, sourceId: source.id });
          if (root) setRoots(current => ({ ...current, [source.id]: root }));
        })}>Choose folder for {source.name}</button>
      </div>)}
      <div className="header-actions">
        <button disabled={busy || (preview.preview.legacy && preview.preview.sources.some(s => !roots[s.id]))} onClick={() => void importCatalog()}>Confirm catalog import</button>
        <button disabled={busy} onClick={() => setPreview(null)}>Cancel import</button>
      </div>
    </section>}
    <button disabled={busy} onClick={() => void action(async () => {
      if (showEvidence) { setShowEvidence(false); return; }
      setEvidence(await call<Evidence[]>('catalog_legacy_evidence')); setShowEvidence(true);
    })}>{showEvidence ? 'Hide legacy evidence' : 'Review legacy evidence'}</button>
    {showEvidence && <div aria-label="Legacy evidence">
      <p>{evidence.length} legacy records. Descriptions below are filename inference, retained independently of new analysis.</p>
      {evidence.slice(0, 100).map(n => <article key={n.sound_id}><p>{n.description}</p><small>Filename inference · sampled coverage: {n.analyzed_seconds ?? 'unknown'} seconds, up to 20 seconds at 12 kHz mono. Full-file facts require new analysis.</small></article>)}
      {evidence.length > 100 && <p>Showing the first 100 records. Export JSON to retain all legacy evidence.</p>}
    </div>}
  </div>;
}
