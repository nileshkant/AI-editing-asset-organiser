import {useEffect, useState} from 'react';
import {call} from '../../api';
type Preferences = {output_device: string | null; imports_paused: boolean};
type Resources = {preferences: Preferences; output_devices: string[]; import_workers: number};
export function RecoverySettings({onError}: {onError: (error: string) => void}) {
  const [resources,setResources] = useState<Resources | null>(null);
  const [busy,setBusy] = useState(false);
  const [confirm,setConfirm] = useState<'restore' | 'cache' | null>(null);
  const [message,setMessage] = useState('');
  useEffect(() => {let active=true; call<Resources>('resource_settings').then(value=>{if(active && value?.preferences && Array.isArray(value.output_devices))setResources(value);}).catch(e=>onError(String(e)));return()=>{active=false};},[onError]);
  const run=async (task:()=>Promise<void>)=>{if(busy)return;setBusy(true);setMessage('');try{await task();}catch(e){onError(String(e));}finally{setBusy(false);}};
  const save=(preferences:Preferences)=>void run(async()=>{await call('save_resource_settings',{preferences});setResources(current=>current ? {...current,preferences}:current);setMessage('Preferences saved. Output changes stop playback; import pause applies after the current job.');});
  const perform=()=>void run(async()=>{
    if(confirm==='cache'){const count=await call<number>('purge_waveform_cache',{confirmed:true});setMessage(`Cleared ${count} waveform cache files. Waveforms rebuild on demand; analysis and tags are retained.`);}
    if(confirm==='restore'){const rollback=await call<string | null>('database_restore',{confirmed:true});if(rollback){setMessage(`Database restored. Rollback backup: ${rollback}. Restart the app, review source catalog warnings and rescan incomplete imports. MCP is stopped; pair again after restart.`);}}
    setConfirm(null);
  });
  return <section aria-label="Recovery and resources">
    <h2>Audio and resources</h2>
    {resources && <>
      <label className="settings-control">Output device <select aria-label="Output device" disabled={busy} value={resources.preferences.output_device || ''} onChange={e=>save({...resources.preferences,output_device:e.target.value || null})}>
        <option value="">System default</option>
        {resources.preferences.output_device && !resources.output_devices.includes(resources.preferences.output_device) && <option value={resources.preferences.output_device}>{resources.preferences.output_device} · unavailable</option>}
        {resources.output_devices.map(name=><option key={name} value={name}>{name}</option>)}
      </select></label>
      <p className="muted">Import workers: {resources.import_workers} · one job at a time, one decoder thread. The current job finishes before a pause takes effect.</p>
      <label className="settings-check"><input type="checkbox" disabled={busy} checked={resources.preferences.imports_paused} onChange={e=>save({...resources.preferences,imports_paused:e.target.checked})}/>Pause queued imports</label>
    </>}
    <h2>Backup and diagnostics</h2>
    <p className="muted">Database backup includes catalog, annotations, clip history, jobs and preferences. It excludes audio, waveform cache, exported clips and temporary MCP credentials. Keep source folders separately.</p>
    <div className="settings-actions">
    <button disabled={busy} onClick={()=>void run(async()=>{const path=await call<string | null>('database_backup');if(path)setMessage(`Database backup saved to ${path}`);})}>Back up database</button>
    <button disabled={busy} onClick={()=>setConfirm('restore')}>Restore database</button>
    <button disabled={busy} onClick={()=>setConfirm('cache')}>Clear waveform cache</button>
    <button disabled={busy} onClick={()=>void run(async()=>{const path=await call<string | null>('support_report');if(path)setMessage(`Redacted diagnostics saved to ${path}`);})}>Export redacted diagnostics</button>
    </div>
    <p className="muted">Diagnostics contain versions, platform, engine availability and counts only. No paths, recordings, filenames, tags, comments, logs or keys. Nothing is uploaded.</p>
    {confirm && <div role="group" aria-label="Confirm recovery action">
      <p>{confirm==='restore'?'Choose a backup to replace current database metadata? Finish or cancel imports and exports first. Playback and MCP stop. A rollback backup is saved first; newer source-folder snapshots are protected by revision checks. Restart after restoration.':'Clear generated waveform cache? Original media, measured profiles, annotations and clip recipes stay intact. Active waveform work finishes first; previews remain available.'}</p>
      <button disabled={busy} onClick={perform}>{confirm==='restore'?'Confirm and choose backup':'Confirm cache clear'}</button>
      <button disabled={busy} onClick={()=>setConfirm(null)}>Cancel recovery action</button>
    </div>}
    {busy && <p role="status">Working on recovery…</p>}
    {message && <p role="status">{message}</p>}
  </section>;
}
