import { useEffect, useState } from 'react';
import { call } from '../../api';
type Client = { id: string; name: string };
type Status = { endpoint: string | null; clients: Client[] };
type Pairing = { client: Client; token: string };
const stopped: Status = { endpoint: null, clients: [] };
export function McpSettings({ onError }: { onError: (error: string) => void }) {
  const [status, setStatus] = useState<Status>(stopped);
  const [port, setPort] = useState('0');
  const [name, setName] = useState('');
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => { let active = true; call<Status>('mcp_status').then(s => { if (active && s) setStatus(s); }).catch(e => { if (active) onError(String(e)); }); return () => { active = false; }; }, [onError]);
  const act = async (command: string, args?: Record<string, unknown>) => {
    if (busy) return;
    setBusy(true);
    try {
      if (command === 'mcp_pair') { setPairing(await call<Pairing>(command, args)); setName(''); setStatus(await call<Status>('mcp_status')); }
      else { setPairing(null); setStatus(await call<Status>(command, args)); }
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
    <p className="muted">Off by default. SoundShelf must stay open. Quitting stops access. Tray mode is unavailable. Clients can check connection status; catalog access arrives in a later ticket.</p>
    <div className="setting-row">
      <span role="status">{status.endpoint ? 'Running' : 'Stopped'}</span>
      {status.endpoint ? <><code>{status.endpoint}</code><button disabled={busy} onClick={() => void act('mcp_stop')}>Stop MCP</button></> : <>
        <label>Port <input aria-label="MCP port" inputMode="numeric" value={port} onChange={e => setPort(e.target.value)} /></label>
        <small>0 selects an available port.</small>
        <button disabled={busy || !validPort} onClick={() => void act('mcp_start', { port: Number(port) })}>Start MCP</button>
      </>}
    </div>
    {status.endpoint && <>
      <form onSubmit={e => { e.preventDefault(); void act('mcp_pair', { name }); }}>
        <label>Client name <input maxLength={80} value={name} onChange={e => setName(e.target.value)} /></label>
        <button disabled={busy || !name.trim()}>Pair client</button>
      </form>
      <p className="muted">Pairings last until MCP stops. Each client has its own credential. Store it privately; it is shown once.</p>
      {pairing && <div>
        <strong>Configuration for {pairing.client.name}</strong>
        <pre aria-label="MCP client configuration">{configuration}</pre>
        <button onClick={() => void copyConfiguration()}>Copy configuration</button>
        <button onClick={() => setPairing(null)}>Hide credential</button>
      </div>}
      <ul aria-label="Paired MCP clients">{status.clients.map(client => <li key={client.id}>{client.name} <button disabled={busy} onClick={() => void act('mcp_revoke', { id: client.id })}>Revoke {client.name}</button></li>)}</ul>
    </>}
  </section>;
}
