import { useEffect, useState } from 'react';
import { call } from '../../api';
import type { GenerationSettings, ImageImportCapabilities } from '../../types';

type Preview = { token: string; preview: { schema: string; sources: { id: string; name: string }[]; sounds: number; images: number; legacy: boolean } };
type Report = { report: { sounds: number; images: number; sources: number; offline_sources: number }; warnings: string[] };
type Evidence = { sound_id: string; description: string; provenance: string; analyzed_seconds: number | null; measurement_scope: string };

export function CatalogData({ onError, active = true }: { onError: (message: string) => void; active?: boolean }) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [roots, setRoots] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [evidence, setEvidence] = useState<Evidence[]>([]);
  const [showEvidence, setShowEvidence] = useState(false);
  // SS-055: generation prompts and private references leave the app only when chosen.
  const [generation, setGeneration] = useState<GenerationSettings | null>(null);
  const [share, setShare] = useState({ prompts: false, private_references: false });
  // Loaded when the pane is first shown, so opening other settings panes makes no call.
  const [shown, setShown] = useState(active);
  if (active && !shown) setShown(true);
  useEffect(() => {
    if (!shown) return;
    let live = true;
    call<ImageImportCapabilities | null>('image_import_capabilities')
      .then(cap => cap?.enabled || cap?.metadata_present ? call<GenerationSettings>('image_generation_settings') : null)
      .then(s => { if (live && s) setGeneration(s); }).catch(() => {});
    return () => { live = false; };
  }, [shown]);
  const saveGeneration = (next: GenerationSettings) => action(async () => {
    setGeneration(await call<GenerationSettings>('set_image_generation_settings', { settings: next }));
    setMessage('Generation metadata settings saved.');
  });
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
    const path = await call<string | null>('catalog_export', generation && format === 'json'
      ? { format, includePrompts: share.prompts, includePrivateReferences: share.private_references }
      : { format });
    if (path) setMessage(`Catalog exported to ${path}`);
  });
  const importCatalog = () => action(async () => {
    if (!preview) return;
    // Preview is consumed on submission, including failed validation.
    const token = preview.token;
    try {
      const result = await call<Report>('catalog_import', { token, confirmed: true });
      const images = result.report.images ? ` and ${result.report.images} images` : '';
      setMessage(`Imported ${result.report.sounds} sounds${images} from ${result.report.sources} sources. ${result.report.offline_sources} sources need relinking. ${result.warnings.join(' ')}`);
    } finally { setPreview(null); }
  });
  return <div className="catalog-data">
    <h2>Data</h2>
    <p className="muted">Transfer your catalog, annotations, saved searches and clips. JSON restores metadata; Markdown provides a readable copy. Audio and images stay in your source folders.</p>
    <div className="header-actions">
      <button disabled={busy || !!preview} onClick={() => void exportCatalog('json')}>Export catalog JSON</button>
      <button disabled={busy || !!preview} onClick={() => void exportCatalog('md')}>Export catalog Markdown</button>
      <button disabled={busy || !!preview} onClick={() => void openImport(false)}>Import catalog</button>
      <button disabled={busy || !!preview} onClick={() => void openImport(true)}>Import legacy catalog</button>
    </div>
    {generation && <fieldset className="catalog-generation">
      <legend>Image generation metadata</legend>
      <p className="muted">Prompts and private references such as local file names are left out of JSON exports unless you include them here. Markdown exports never include them.</p>
      <label><input type="checkbox" checked={share.prompts} disabled={busy} onChange={e => setShare({ ...share, prompts: e.target.checked })} /> Include prompts in this JSON export</label>
      <label><input type="checkbox" checked={share.private_references} disabled={busy} onChange={e => setShare({ ...share, private_references: e.target.checked })} /> Include private references in this JSON export</label>
      <label htmlFor="generation-search">Search generation metadata</label>
      <select id="generation-search" value={generation.search} disabled={busy} onChange={e => void saveGeneration({ ...generation, search: e.target.value as GenerationSettings['search'] })}>
        <option value="exclude">Do not search it</option>
        <option value="metadata">Everything except prompts</option>
        <option value="prompts">Also search prompts</option>
      </select>
      <p className="muted">Image search is not available yet. This choice is saved and applied when it is.</p>
      <label><input type="checkbox" checked={generation.snapshot.prompts} disabled={busy} onChange={e => void saveGeneration({ ...generation, snapshot: { ...generation.snapshot, prompts: e.target.checked } })} /> Include prompts in folder catalog snapshots</label>
      <label><input type="checkbox" checked={generation.snapshot.private_references} disabled={busy} onChange={e => void saveGeneration({ ...generation, snapshot: { ...generation.snapshot, private_references: e.target.checked } })} /> Include private references in folder catalog snapshots</label>
    </fieldset>}
    {busy && <p role="status">Working on catalog…</p>}
    {message && <p role="status">{message}</p>}
    {preview && <section className="catalog-preview" aria-label="Catalog import preview">
      <h3>{preview.preview.legacy ? 'Review legacy import' : 'Review catalog import'}</h3>
      <p>{preview.preview.sounds.toLocaleString()} sounds{preview.preview.images ? ` · ${preview.preview.images.toLocaleString()} images` : ''} · {preview.preview.sources.length} sources</p>
      <p>Choose a local folder for each source. Existing catalog entries are preserved; conflicting identities or overlapping sources stop the import.</p>
      {preview.preview.legacy
        ? <p>Map every source before importing. Legacy descriptions are filename inference. Old measurements cover only the first 20 seconds at 12 kHz mono. Full-file analysis will run in the background.</p>
        : <p>Unmapped sources are imported offline. Relink them in Sources when their folders are available. Import reuses stored analysis without reading audio or image files; run Check files if mapped files may have changed.</p>}
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
