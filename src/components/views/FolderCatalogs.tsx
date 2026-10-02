import { useEffect, useState } from 'react';
import { call } from '../../api';
import type { Source } from '../../types';
type State = {source_id: string; dirty: boolean; error: string};
export function FolderCatalogs({roots, onError}: {roots: Source[]; onError: (message: string) => void}) {
  const [states,setStates] = useState<State[]>([]);
  const [busy,setBusy] = useState(false);
  const [recoveryPath,setRecoveryPath]=useState<string|null>(null);
  const [rebuild,setRebuild] = useState<Source|null>(null);
  const load = async () => {const result=await call<State[]>('folder_catalog_states');setStates(Array.isArray(result)?result:[]);};
  useEffect(()=>{if(!roots.some(root=>root.scope!=='files'))return;let active=true;void call<State[]>('folder_catalog_states').then(result=>{if(active)setStates(Array.isArray(result)?result:[]);}).catch(e=>onError(String(e)));return()=>{active=false;};},[onError,roots]);
  const run = async (source: Source, reset = false) => {
    setBusy(true);
    try {await call(reset?'rebuild_folder_catalog':'retry_folder_catalog',{id:source.id,...(reset?{confirmed:true}:{})});setRebuild(null);await load();}
    catch(e){onError(String(e));}finally{setBusy(false);}
  };
  return <section aria-label="Folder metadata catalogs">
    <h2>Folder metadata catalogs</h2>
    <p className="muted">Audio stays in its source folder. A metadata catalog saves analysis, tags and clips there. Reimport checks listed files; Rescan discovers new files and verifies content. Files with unchanged size and timestamp reuse analysis.</p>
    {roots.filter(root=>root.scope!=='files').map(root=>{
      const status=states.find(state=>state.source_id===root.id);
      return <div className="setting-row" key={root.id}><div><strong>{root.name}</strong><p>{status?.dirty?'Snapshot needs retry':status?'Snapshot saved':'Snapshot will be created after import'}</p>{status?.error&&<p role="status">{status.error}</p>}</div><div className="header-actions"><button disabled={busy||!root.available} onClick={()=>void run(root)}>Save metadata</button><button disabled={busy||!root.available} onClick={()=>setRebuild(root)}>Rebuild catalog</button></div></div>;
    })}
    <button disabled={busy} onClick={()=>{void call<string|null>('choose_folder').then(path=>setRecoveryPath(path)).catch(e=>onError(String(e)));}}>Recover a folder catalog</button>
    {recoveryPath&&<section aria-label="Confirm folder recovery"><p>Rebuild metadata in {recoveryPath}? The previous catalog will be kept as a backup and audio analyzed again.</p><button disabled={busy} onClick={()=>{setBusy(true);void call('rebuild_import_folder',{path:recoveryPath,confirmed:true}).then(()=>setRecoveryPath(null)).catch(e=>onError(String(e))).finally(()=>setBusy(false));}}>Confirm folder recovery</button><button disabled={busy} onClick={()=>setRecoveryPath(null)}>Cancel recovery</button></section>}
    {rebuild&&<section aria-label="Confirm catalog rebuild"><p>Rebuild metadata for {rebuild.name}? The current catalog is preserved as a backup. The app will scan and verify audio again; local annotations stay in the database.</p><button disabled={busy} onClick={()=>void run(rebuild,true)}>Confirm rebuild</button><button disabled={busy} onClick={()=>setRebuild(null)}>Cancel rebuild</button></section>}
  </section>;
}
