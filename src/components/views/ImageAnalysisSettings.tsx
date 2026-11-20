import { useCallback, useEffect, useRef, useState } from 'react';
import { call } from '../../api';
import type {
  EnrichConsent, EnrichDisclosure, EnrichEstimate, EnrichItem, EnrichJob, ImageImportCapabilities, ImageSourceStatus, ProviderView,
} from '../../types';

const pauseText: Record<string, string> = {
  PROVIDER_REMOVED: 'The provider was removed.', PROVIDER_CHANGED: 'The provider settings changed since you agreed.',
  PROVIDER_NOT_READY: 'The provider must pass its connection test again.', SOURCE_OFFLINE: 'The folder is offline.',
  REQUEST_CEILING: 'The request limit for this batch was reached.', UNIT_CEILING: 'The usage limit for this batch was reached.',
  RATE_LIMITED: 'The provider asked to slow down.', PROVIDER_AUTH: 'The provider rejected the key.', QUOTA_EXCEEDED: 'The provider reports no quota left.',
  MODEL_NOT_FOUND: 'The provider does not know this model.', CONSENT_CHANGED: 'The permission changed.', CONSENT_REVOKED: 'Analysis was stopped for this folder.',
};
const stateText: Record<EnrichJob['state'], string> = {
  queued: 'Waiting', running: 'Running', paused: 'Paused', complete: 'Finished', failed: 'Failed', cancelled: 'Cancelled',
};
const ready = (p: ProviderView) => p.capabilities.tested_revision === p.revision && p.capabilities.image_input === 'supported' && p.capabilities.structured_output === 'supported';
const active = (j: EnrichJob) => j.state === 'queued' || j.state === 'running';

function Job({ job, sourceName, run }: { job: EnrichJob; sourceName: string; run: (task: () => Promise<unknown>) => Promise<void> }) {
  const [items, setItems] = useState<EnrichItem[] | null>(null);
  const [accepted, setAccepted] = useState<Record<string, boolean>>({});
  const c = job.counts, u = job.usage;
  const total = c.pending + c.running + c.done + c.reused + c.failed + c.unknown_outcome + c.stale + c.revoked;
  const finished = c.done + c.reused + c.failed + c.unknown_outcome + c.stale + c.revoked;
  const review = c.unknown_outcome + c.failed;
  const loadItems = () => call<EnrichItem[]>('image_enrichment_items', { jobId: job.id, offset: 0, limit: 100 }).then(setItems);
  return <article className="job-row" aria-label={`Analysis batch for ${sourceName}`}>
    <div className="job-heading"><strong>{sourceName}</strong><span role="status">{stateText[job.state]}{job.error_code && pauseText[job.error_code] ? ` · ${pauseText[job.error_code]}` : ''}</span></div>
    <progress aria-label={`Progress for ${sourceName}`} max={total || 1} value={finished} />
    <p>{c.done} analyzed · {c.reused} reused from identical images · {c.pending + c.running} waiting · {c.failed} failed · {c.stale} skipped because they changed · {c.revoked} stopped</p>
    <p className="muted">Requests sent: {u.requests} of at most {job.max_requests}. Usage: {u.used_units} of at most {job.max_units} units{u.estimated_entries > 0 ? ' (partly estimated, the provider did not report all of it)' : ''}.</p>
    {c.unknown_outcome > 0 && <p role="alert">{c.unknown_outcome} request{c.unknown_outcome === 1 ? '' : 's'} may or may not have reached the provider, so up to {u.unknown_units} units might be charged. They are not retried unless you choose to.</p>}
    <div className="image-import-actions">
      {active(job) && <button onClick={() => void run(() => call('image_enrichment_cancel', { jobId: job.id }))}>Cancel batch for {sourceName}</button>}
      {job.state === 'paused' && <button onClick={() => void run(() => call('image_enrichment_resume', { jobId: job.id }))}>Resume batch for {sourceName}</button>}
      {review > 0 && !items && <button onClick={() => void loadItems()}>Review {review} image{review === 1 ? '' : 's'} that need attention</button>}
    </div>
    {items && <ul className="image-import-items">{items.filter(i => i.state === 'unknown_outcome' || i.state === 'failed').map(i => {
      const unknown = i.state === 'unknown_outcome';
      return <li key={i.asset_id}>
        <span className="path-text">{i.asset_id}</span> · {unknown ? 'outcome unknown' : `failed${i.error_code ? ` (${i.error_code})` : ''}`}
        {unknown && <label className="settings-check"><input type="checkbox" checked={!!accepted[i.asset_id]} onChange={e => setAccepted({ ...accepted, [i.asset_id]: e.target.checked })} />I accept a possible second charge</label>}
        <button disabled={unknown && !accepted[i.asset_id]} aria-label={`Retry ${i.asset_id}`}
          onClick={() => void run(async () => { await call('image_enrichment_retry', { jobId: job.id, assetId: i.asset_id, acceptPossibleCharge: unknown }); await loadItems(); })}>Retry</button>
      </li>;
    })}</ul>}
  </article>;
}

export function ImageAnalysisSettings({ onError }: { onError: (message: string) => void }) {
  const [available, setAvailable] = useState(false);
  const [disclosure, setDisclosure] = useState<EnrichDisclosure | null>(null);
  const [sources, setSources] = useState<ImageSourceStatus[]>([]);
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [consents, setConsents] = useState<EnrichConsent[]>([]);
  const [jobs, setJobs] = useState<EnrichJob[]>([]);
  const [sourceId, setSourceId] = useState('');
  const [providerId, setProviderId] = useState('');
  const [analyzeNew, setAnalyzeNew] = useState(false);
  const [agreed, setAgreed] = useState(false);
  const [estimate, setEstimate] = useState<EnrichEstimate | null>(null);
  const [maxRequests, setMaxRequests] = useState('');
  const [maxUnits, setMaxUnits] = useState('');
  const [busy, setBusy] = useState(false);
  const latest = useRef(0);
  const fail = useCallback((e: unknown) => onError(String(e)), [onError]);

  const refresh = useCallback(async () => {
    const ticket = ++latest.current;
    try {
      const [c, j] = await Promise.all([call<EnrichConsent[]>('image_enrichment_consents'), call<EnrichJob[]>('image_enrichment_jobs')]);
      if (ticket === latest.current) { setConsents(c); setJobs(j); }
    } catch (e) { fail(e); }
  }, [fail]);
  useEffect(() => {
    let live = true;
    (async () => {
      const cap = await call<ImageImportCapabilities | null>('image_import_capabilities').catch(() => null);
      if (!cap?.enabled || !live) return;
      setAvailable(true);
      try {
        const [d, s, p] = await Promise.all([call<EnrichDisclosure>('image_enrichment_disclosure'), call<ImageSourceStatus[]>('image_source_statuses'), call<ProviderView[]>('ai_providers')]);
        if (!live) return;
        setDisclosure(d); setSources(s); setProviders(p);
        setSourceId(id => id || s[0]?.source_id || '');
        setProviderId(id => id || p.find(ready)?.id || '');
        await refresh();
      } catch (e) { if (live) fail(e); }
    })();
    return () => { live = false; };
  }, [refresh, fail]);
  const working = jobs.some(active);
  useEffect(() => {
    if (!working) return;
    const timer = setInterval(() => void refresh(), 2000);
    return () => clearInterval(timer);
  }, [working, refresh]);

  const run = async (task: () => Promise<unknown>) => {
    setBusy(true);
    try { await task(); } catch (e) { fail(e); } finally { setBusy(false); await refresh(); }
  };
  if (!available) return null;

  const source = sources.find(s => s.source_id === sourceId);
  const provider = providers.find(p => p.id === providerId);
  const consent = consents.find(c => c.source_id === sourceId);
  const consentProvider = consent && providers.find(p => p.id === consent.provider_id);
  const stale = !!consent && (!consentProvider || consentProvider.revision !== consent.provider_revision || !ready(consentProvider));
  const nameOf = (id: string) => sources.find(s => s.source_id === id)?.name ?? 'Removed folder';
  const ceilingsValid = Number.isInteger(Number(maxRequests)) && Number(maxRequests) > 0 && Number.isInteger(Number(maxUnits)) && Number(maxUnits) > 0;

  const grant = () => run(async () => {
    if (!provider) return;
    await call('image_enrichment_grant', { request: { source_id: sourceId, provider_id: provider.id, provider_revision: provider.revision, analyze_new: analyzeNew } });
    setAgreed(false); setEstimate(null);
  });
  const doEstimate = () => run(async () => {
    if (!consent) return;
    const next = await call<EnrichEstimate>('image_enrichment_estimate', { request: { source_id: sourceId, provider_id: consent.provider_id, provider_revision: consent.provider_revision, kind: 'backfill', asset_ids: null } });
    setEstimate(next); setMaxRequests(String(next.suggested_max_requests)); setMaxUnits(String(next.suggested_max_units));
  });
  const start = () => run(async () => {
    if (!consent) return;
    await call('image_enrichment_start', { request: { source_id: sourceId, provider_id: consent.provider_id, provider_revision: consent.provider_revision, kind: 'backfill', asset_ids: null, max_requests: Number(maxRequests), max_units: Number(maxUnits) } });
    setEstimate(null);
  });

  return <section aria-label="Image analysis">
    <h2>Image analysis</h2>
    <p className="muted">Optional. Importing, browsing, tags and search never need it. Analysis sends copies of images to the AI provider you set up above, only for folders you allow, and never starts by itself unless you choose to analyze new imports.</p>
    {disclosure && <div role="note" aria-label="What is sent">
      <strong>{disclosure.purpose}</strong>
      <p>Sent: {disclosure.sends}</p>
      <p>Never sent:</p>
      <ul>{disclosure.never_sends.map(n => <li key={n}>{n}</li>)}</ul>
      <p className="muted">Images already sent cannot be taken back, and stopping analysis does not undo them.</p>
    </div>}
    {!sources.length && <p>Import an image folder first. Then you can allow analysis for it here.</p>}
    {sources.length > 0 && <>
      <label>Folder <select value={sourceId} onChange={e => { setSourceId(e.target.value); setEstimate(null); setAgreed(false); }}>
        {sources.map(s => <option key={s.source_id} value={s.source_id}>{s.name}</option>)}</select></label>
      {consent && <div aria-label="Current permission">
        <p role="status">Allowed for {source?.name}: provider {consentProvider?.label ?? 'removed'} · model {consent.model}{consent.analyze_new ? ' · new imports are analyzed automatically' : ''}.</p>
        {stale && <p role="alert">The provider changed or has not passed its test since you agreed. Nothing is sent until you allow it again below.</p>}
        <button disabled={busy} onClick={() => void run(async () => { await call('image_enrichment_revoke', { sourceId }); setEstimate(null); })}>Stop allowing analysis for {source?.name}</button>
      </div>}
      {(!consent || stale) && <fieldset>
        <legend>Allow analysis for {source?.name}</legend>
        <label>Provider <select value={providerId} onChange={e => setProviderId(e.target.value)}>
          <option value="">Choose a provider</option>
          {providers.map(p => <option key={p.id} value={p.id} disabled={!ready(p)}>{p.label} · {p.model}{ready(p) ? '' : ' (test it first)'}</option>)}</select></label>
        {!providers.some(ready) && <p>No provider has passed its image and structured-output test yet. Add one and test it above.</p>}
        <label className="settings-check"><input type="checkbox" checked={analyzeNew} onChange={e => setAnalyzeNew(e.target.checked)} />Also analyze images I import into this folder later (at most 200 per import)</label>
        <label className="settings-check"><input type="checkbox" checked={agreed} onChange={e => setAgreed(e.target.checked)} />I understand copies of these images will be sent to {provider?.label ?? 'the chosen provider'}{provider && provider.locality === 'remote' ? ' over the internet' : ''}</label>
        <button disabled={busy || !provider || !agreed} onClick={() => void grant()}>Allow analysis</button>
      </fieldset>}
      {consent && !stale && <div>
        <button disabled={busy} onClick={() => void doEstimate()}>Estimate analysis of {source?.name}</button>
        {estimate && <div aria-label="Estimate">
          {estimate.to_send === 0 ? <p>Nothing to send: {estimate.already_done} already analyzed, {estimate.reusable} can reuse an identical image’s result, {estimate.already_queued} waiting in another batch, {estimate.not_ready} not ready.</p> : <>
            <p>{estimate.to_send} image{estimate.to_send === 1 ? '' : 's'} would be sent. {estimate.reusable} reuse an identical image’s result without a request, {estimate.already_done} are already analyzed, {estimate.not_ready} are not ready.</p>
            <p className="muted">About {estimate.estimated_units} {estimate.unit} in total (roughly {estimate.units_per_request} each). {estimate.cost}</p>
            <label>Most requests to send <input inputMode="numeric" value={maxRequests} onChange={e => setMaxRequests(e.target.value)} /></label>
            <label>Most {estimate.unit} to use <input inputMode="numeric" value={maxUnits} onChange={e => setMaxUnits(e.target.value)} /></label>
            <p className="muted">The batch pauses when either limit is reached, before anything beyond it is sent.</p>
            <button disabled={busy || !ceilingsValid} onClick={() => void start()}>Start analysis</button></>}
        </div>}
      </div>}
    </>}
    {jobs.length > 0 && <h3>Batches</h3>}
    {jobs.map(j => <Job key={j.id} job={j} sourceName={nameOf(j.source_id)} run={run} />)}
  </section>;
}
