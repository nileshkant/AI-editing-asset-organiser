import { useEffect, useState } from 'react';
import { call } from '../../api';
import type { Source } from '../../types';
type Access = { source_ids: string[]; read: boolean; edit: boolean; export: boolean; paths: boolean };
const defaultAccess: Access = { source_ids: [], read: true, edit: false, export: false, paths: false };
type Client = { id: string; name: string; access?: Access };
type Status = { endpoint: string | null; clients: Client[] };
type Pairing = { client: Client; token: string };
const stopped: Status = { endpoint: null, clients: [] };
export function McpSettings({ onError, roots = [] }: { onError: (error: string) => void; roots?: Source[] }) {
  const [status, setStatus] = useState<Status>(stopped);
  const [port, setPort] = useState('0');
  const [name, setName] = useState('');
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [busy, setBusy] = useState(false);
  const [access, setAccess] = useState<Access>(defaultAccess);
  const [destination, setDestination] = useState<{ clientId: string; id: string; path: string } | null>(null);
  const approveDestination = async (clientId: string, format: 'wav' | 'flac') => {
    if (busy) return; setBusy(true);
    try { const grant = await call<{ id: string; path: string } | null>('mcp_approve_destination', { clientId, format }); if (grant) setDestination({ clientId, ...grant }); }
    catch (e) { onError(String(e)); } finally { setBusy(false); }
  };
  useEffect(() => { let active = true; call<Status>('mcp_status').then(s => { if (active && s) setStatus(s); }).catch(e => { if (active) onError(String(e)); }); return () => { active = false; }; }, [onError]);
  const act = async (command: string, args?: Record<string, unknown>) => {
    if (busy) return;
    setBusy(true);
    try {
      if (command === 'mcp_pair') { setPairing(await call<Pairing>(command, args)); setName(''); setStatus(await call<Status>('mcp_status')); }
      else { setDestination(null); setPairing(null); setStatus(await call<Status>(command, args)); }
    } catch (e) { onError(String(e)); } finally { setBusy(false); }
  };
  const configuration = pairing ? JSON.stringify({ mcpServers: { soundshelf: { url: status.endpoint, headers: { Authorization: `Bearer ${pairing.token}` } } } }, null, 2) : '';
  const copyConfiguration = async () => {
    try { if (!navigator.clipboard) throw new Error(); await navigator.clipboard.writeText(configuration); }
    catch { onError('Could not copy configuration. Select and copy the displayed text.'); }
  };
  const validPort = /^\d{1,5}$/.test(port) && Number(port) <= 65535;
  return <section aria-label="MCP access">
    <h2>Agent access · MCP</h2>
    <p className="muted">Off by default. SoundShelf must stay open. Quitting stops access. Tray mode is unavailable. Give each client access to specific sources. Editing, exporting and machine paths require separate permission.</p>
    <div className="setting-row">
      <span role="status">{status.endpoint ? 'Running' : 'Stopped'}</span>
      {status.endpoint ? <><code>{status.endpoint}</code><button disabled={busy} onClick={() => void act('mcp_stop')}>Stop MCP</button></> : <>
        <label>Port <input aria-label="MCP port" inputMode="numeric" value={port} onChange={e => setPort(e.target.value)} /></label>
        <small>0 selects an available port.</small>
        <button disabled={busy || !validPort} onClick={() => void act('mcp_start', { port: Number(port) })}>Start MCP</button>
      </>}
    </div>
    {status.endpoint && <>
      <form onSubmit={e => { e.preventDefault(); void act('mcp_pair', { name, access }); }}>
        <label>Client name <input maxLength={80} value={name} onChange={e => setName(e.target.value)} /></label>
        <fieldset disabled={busy}><legend>Client permissions</legend>
          <label><input type="checkbox" checked={access.read} onChange={e => setAccess(e.target.checked ? defaultAccess : { ...defaultAccess, read: false })} />Read catalog</label>
          {(['edit', 'export', 'paths'] as const).map(key => <label key={key}><input type="checkbox" disabled={!access.read} checked={access[key]} onChange={e => setAccess(a => ({ ...a, [key]: e.target.checked }))} />{key === 'edit' ? 'Edit annotations and clips' : key === 'export' ? 'Export clips to approved destinations' : 'Reveal local paths'}</label>)}
          <fieldset disabled={!access.read}><legend>Allowed sources</legend>
            {roots.map(source => <label key={source.id}><input type="checkbox" checked={access.source_ids.includes(source.id)} onChange={e => setAccess(a => ({ ...a, source_ids: e.target.checked ? [...a.source_ids, source.id] : a.source_ids.filter(id => id !== source.id) }))} />{source.name}</label>)}
            {!roots.length && <p className="muted">No sources available. Catalog results will be empty.</p>}
          </fieldset>
        </fieldset>
        <button disabled={busy || !name.trim()}>Pair client</button>
      </form>
      <p className="muted">Pairings last until MCP stops. Each client has its own credential. Store it privately; it is shown once.</p>
      {pairing && <div>
        <strong>Configuration for {pairing.client.name}</strong>
        <pre aria-label="MCP client configuration">{configuration}</pre>
        <button onClick={() => void copyConfiguration()}>Copy configuration</button>
        <button onClick={() => setPairing(null)}>Hide credential</button>
      </div>}
      {destination && <p role="status">Approved for {status.clients.find(c => c.id === destination.clientId)?.name}: <code>{destination.path}</code> · Destination ID <code>{destination.id}</code>. Give this ID to that client for one export.</p>}
      <ul aria-label="Paired MCP clients">{status.clients.map(client => <li key={client.id}>{client.name} · {client.access?.source_ids.length || 0} sources · {!client.access?.read ? 'Status only' : client.access?.edit ? 'Edit enabled' : 'Read only'}
        {client.access?.export && <><button disabled={busy} onClick={() => void approveDestination(client.id, 'wav')}>Approve WAV destination for {client.name}</button><button disabled={busy} onClick={() => void approveDestination(client.id, 'flac')}>Approve FLAC destination for {client.name}</button></>} <button disabled={busy} onClick={() => void act('mcp_revoke', { id: client.id })}>Revoke {client.name}</button></li>)}</ul>
    </>}
  </section>;
}
