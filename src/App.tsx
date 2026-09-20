import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { AudioLines, Library, Settings2, ShieldCheck, FolderOpen, Activity } from 'lucide-react';

type AppInfo = { version: string; data_directory: string; desktop: boolean };
export function App() {
  const [page, setPage] = useState<'library' | 'settings'>('library');
  const [info, setInfo] = useState<AppInfo>();
  const [error, setError] = useState('');
  useEffect(() => { if (isTauri()) invoke<AppInfo>('app_info').then(setInfo).catch(e => setError(String(e))); }, []);
  return <div className="app-shell">
    <aside className="sidebar">
      <div className="brand"><AudioLines size={25}/><strong>SoundShelf</strong></div>
      <nav aria-label="Main navigation">
        <button className={page === 'library' ? 'nav active' : 'nav'} onClick={() => setPage('library')}><Library size={18}/>Library</button>
        <button className={page === 'settings' ? 'nav active' : 'nav'} onClick={() => setPage('settings')}><Settings2 size={18}/>Settings</button>
      </nav>
      <div className="sidebar-bottom"><ShieldCheck size={15}/><span>Local workspace</span><span className="status-dot"/></div>
    </aside>
    <main>
      <header className="page-header"><div><p className="eyebrow">YOUR AUDIO WORKSPACE</p><h1>{page === 'library' ? 'Library' : 'Settings'}</h1></div><span className="build-label">Development build</span></header>
      {error && <p role="alert" className="error">{error}</p>}
      {page === 'library' ? <section className="empty"><FolderOpen size={48} strokeWidth={1}/><h2>No sounds yet</h2><p>Folder import is not available in this foundation build.</p></section> : <section className="settings-body">
        <h2>Workspace</h2><div className="setting-row"><span>Application</span><span>SoundShelf {info?.version ?? '0.1.0'}</span></div>
        <div className="setting-row"><span>Data directory</span><code>{info?.data_directory ?? 'Available in the desktop app'}</code></div>
        <h2>Intelligence</h2><div className="setting-row"><span>AI features</span><span className="muted">Disabled</span></div>
      </section>}
      <footer className="transport-empty"><AudioLines size={24}/><span>No audio selected</span><Activity size={16}/></footer>
    </main>
  </div>;
}
