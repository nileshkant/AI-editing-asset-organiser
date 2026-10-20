import { useEffect, useId, useState } from 'react';
import { call } from '../../api';
import type { CapabilityState, ProviderDraft, ProviderKind, ProviderTestReport, ProviderView } from '../../types';

const kinds: { value: ProviderKind; label: string; endpoint: string }[] = [
  { value: 'openai', label: 'OpenAI', endpoint: 'https://api.openai.com/v1' },
  { value: 'anthropic', label: 'Anthropic', endpoint: 'https://api.anthropic.com/v1' },
  { value: 'gemini', label: 'Google Gemini', endpoint: 'https://generativelanguage.googleapis.com/v1beta' },
  { value: 'openai_compatible', label: 'OpenAI-compatible server', endpoint: '' },
];
const kindLabel = (kind: ProviderKind) => kinds.find(k => k.value === kind)?.label ?? kind;
const stateLabel: Record<CapabilityState, string> = { untested: 'Not tested', supported: 'Supported', unsupported: 'Not supported', inconclusive: 'Inconclusive' };
const checkLabel = { model_access: 'Key and model', image_input: 'Image input', structured_output: 'Structured output', embeddings: 'Embeddings' };
const locality = (p: ProviderView) => p.locality === 'loopback' ? 'This computer' : p.locality === 'private' ? 'Private network' : p.locality === 'remote' ? 'Internet (HTTPS)' : 'Endpoint needs attention';
const keyLabel = (p: ProviderView) => p.key_storage === 'keychain' ? 'Key in system keychain'
  : p.key_storage === 'session' ? (p.has_key ? 'Key for this session only' : 'Session key ended; enter it again')
  : p.kind === 'openai_compatible' ? 'No key (allowed for local servers)' : 'No key';
const empty: ProviderDraft = { kind: 'openai', label: '', base_url: null, model: '', allow_local: false };

function ProviderForm({ initial, busy, submitLabel, onSubmit, onCancel }: {
  initial: ProviderDraft; busy: boolean; submitLabel: string; onSubmit: (draft: ProviderDraft) => void; onCancel?: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const id = useId();
  const kind = kinds.find(k => k.value === draft.kind)!;
  const valid = draft.label.trim() && draft.model.trim() && (draft.kind !== 'openai_compatible' || draft.base_url?.trim());
  return <form aria-label={submitLabel} onSubmit={e => { e.preventDefault(); if (valid) onSubmit({ ...draft, base_url: draft.base_url?.trim() || null }); }}>
    <fieldset disabled={busy}>
      <label className="settings-control" htmlFor={`${id}-kind`}>Provider</label>
      <select id={`${id}-kind`} value={draft.kind} onChange={e => setDraft({ ...draft, kind: e.target.value as ProviderKind })}>
        {kinds.map(k => <option key={k.value} value={k.value}>{k.label}</option>)}
      </select>
      <label className="settings-control">Name <input maxLength={80} value={draft.label} onChange={e => setDraft({ ...draft, label: e.target.value })} /></label>
      <label className="settings-control">Model <input maxLength={200} value={draft.model} onChange={e => setDraft({ ...draft, model: e.target.value })} /></label>
      <label className="settings-control">Endpoint URL <input type="url" maxLength={2048} placeholder={kind.endpoint || 'http://127.0.0.1:11434/v1'}
        value={draft.base_url ?? ''} onChange={e => setDraft({ ...draft, base_url: e.target.value })} /></label>
      {kind.endpoint && <small className="muted">Leave empty for {kind.endpoint}.</small>}
      <label className="settings-control"><input type="checkbox" checked={draft.allow_local} onChange={e => setDraft({ ...draft, allow_local: e.target.checked })} />
        Allow an endpoint on this computer or private network (plain HTTP allowed there)</label>
      <button type="submit" disabled={!valid}>{submitLabel}</button>
      {onCancel && <button type="button" onClick={onCancel}>Cancel</button>}
    </fieldset>
  </form>;
}

function Provider({ provider, busy, run }: { provider: ProviderView; busy: boolean; run: (command: string, args: Record<string, unknown>) => Promise<unknown> }) {
  const [editing, setEditing] = useState(false);
  const [key, setKey] = useState('');
  const [store, setStore] = useState<'keychain' | 'session'>('keychain');
  const [embeddings, setEmbeddings] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [report, setReport] = useState<ProviderTestReport | null>(null);
  const caps = provider.capabilities;
  const base = { id: provider.id, revision: provider.revision };
  const name = provider.label;
  const keyNeeded = provider.kind !== 'openai_compatible' && !provider.has_key;
  return <article className="setting-row" aria-label={`Provider ${name}`}>
    <div className="folder-detail">
      <strong>{name}</strong>
      <small className="muted">{kindLabel(provider.kind)} · model <code>{provider.model}</code></small>
      <code>{provider.base_url}</code>
      <small className="muted">{locality(provider)} · {keyLabel(provider)}</small>
      <dl aria-label={`Tested capabilities of ${name}`}>
        <dt>Image input</dt><dd>{stateLabel[caps.image_input]}</dd>
        <dt>Structured output</dt><dd>{stateLabel[caps.structured_output]}</dd>
        <dt>Embeddings</dt><dd>{stateLabel[caps.embeddings]}</dd>
        {caps.model_version && <><dt>Served model</dt><dd><code>{caps.model_version}</code></dd></>}
      </dl>
      {report && <ul aria-label={`Connection test results for ${name}`}>
        {report.steps.map(s => <li key={s.check}>{checkLabel[s.check]}: {s.passed ? 'passed' : `failed · ${s.error?.message ?? 'unknown error'}`}</li>)}
        {report.usage.input_tokens != null && <li>Usage reported: {report.usage.input_tokens} input and {report.usage.output_tokens ?? 0} output tokens</li>}
      </ul>}
    </div>
    <div className="header-actions">
      <label className="settings-control"><input type="checkbox" checked={embeddings} onChange={e => setEmbeddings(e.target.checked)} />Also test embeddings</label>
      <button disabled={busy || keyNeeded} title={keyNeeded ? 'Add a key first' : undefined}
        onClick={() => void run('ai_provider_test', { ...base, embeddings }).then(r => r && setReport(r as ProviderTestReport))}>Test connection to {name}</button>
      <button disabled={busy} onClick={() => setEditing(!editing)} aria-expanded={editing}>Edit {name}</button>
      {provider.key_storage !== 'none' && <button disabled={busy} onClick={() => void run('ai_provider_clear_key', base)}>Remove key for {name}</button>}
      {removing ? <>
        <button className="settings-danger" disabled={busy} onClick={() => void run('ai_provider_delete', { ...base, confirmed: true })}>Confirm removing {name}</button>
        <button disabled={busy} onClick={() => setRemoving(false)}>Keep {name}</button>
      </> : <button className="settings-danger" disabled={busy} onClick={() => setRemoving(true)}>Remove {name}</button>}
    </div>
    <form aria-label={`API key for ${name}`} onSubmit={e => {
      e.preventDefault();
      void run('ai_provider_set_key', { ...base, key, store }).then(r => { if (r) setKey(''); });
    }}>
      <fieldset disabled={busy}>
        <legend>{provider.key_storage === 'none' ? 'Add API key' : 'Replace API key'}</legend>
        <label className="settings-control">API key <input type="password" autoComplete="off" spellCheck={false} value={key} onChange={e => setKey(e.target.value)} /></label>
        <label><input type="radio" name={`${provider.id}-store`} checked={store === 'keychain'} onChange={() => setStore('keychain')} />Save in the system keychain</label>
        <label><input type="radio" name={`${provider.id}-store`} checked={store === 'session'} onChange={() => setStore('session')} />Use for this session only</label>
        <button type="submit" disabled={key.trim().length < 8}>Save key for {name}</button>
      </fieldset>
    </form>
    {editing && <ProviderForm busy={busy} submitLabel={`Save ${name}`} onCancel={() => setEditing(false)}
      initial={{ kind: provider.kind, label: provider.label, base_url: provider.base_url, model: provider.model, allow_local: provider.allow_local }}
      onSubmit={draft => void run('ai_provider_update', { ...base, draft }).then(r => { if (r) setEditing(false); })} />}
  </article>;
}

/** SS-049: provider profiles and keys. Nothing here analyzes images. */
export function AiProviderSettings({ onError }: { onError: (error: string) => void }) {
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [busy, setBusy] = useState(false);
  const [adding, setAdding] = useState(false);
  const [notice, setNotice] = useState('');
  // Older desktop builds and test doubles may answer with something else; show an empty list.
  const list = (p: unknown) => Array.isArray(p) ? p as ProviderView[] : [];
  const refresh = async () => setProviders(list(await call<unknown>('ai_providers')));
  useEffect(() => {
    let active = true;
    call<unknown>('ai_providers').then(p => { if (active) setProviders(list(p)); }).catch(e => { if (active) onError(String(e)); });
    return () => { active = false; };
  }, [onError]);
  const run = async (command: string, args: Record<string, unknown>) => {
    if (busy) return null;
    setBusy(true); setNotice('');
    try {
      const result = await call<unknown>(command, args);
      await refresh();
      setNotice(command === 'ai_provider_test' ? 'Connection test finished.' : 'Provider settings saved.');
      return result ?? true;
    } catch (e) {
      onError(String(e));
      // A conflict means another change landed first; show the current settings.
      await refresh().catch(() => undefined);
      return null;
    } finally { setBusy(false); }
  };
  return <section aria-label="AI providers">
    <h2>AI providers</h2>
    <p className="muted">Optional. Adding a provider or key sends nothing. Only <strong>Test connection</strong> contacts the provider, and it sends a generated 16×16 test image and fixed text, never your files or catalog. Image analysis will ask separately which sources may be sent. Keys stay in the system keychain or, for this session only, in memory; they are never shown again or written to the catalog, backups or exports.</p>
    <p role="status">{notice}</p>
    {providers.length ? providers.map(p => <Provider key={p.id} provider={p} busy={busy} run={run} />) : <p className="muted">No AI providers configured.</p>}
    {adding
      ? <ProviderForm busy={busy} submitLabel="Add provider" initial={empty} onCancel={() => setAdding(false)}
          onSubmit={draft => void run('ai_provider_create', { draft }).then(r => { if (r) setAdding(false); })} />
      : <button disabled={busy || providers.length >= 16} onClick={() => setAdding(true)}>Add AI provider</button>}
  </section>;
}
