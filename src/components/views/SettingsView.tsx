import React, { memo, useState, useRef } from 'react';
import { RefreshCw } from 'lucide-react';
import { call } from '../../api';
import { McpSettings } from './McpSettings';
import { CatalogData } from './CatalogData';
import type { AppInfo, Source } from '../../types';

interface SettingsViewProps {
  roots: Source[];
  info?: AppInfo;
  onRescan: (id: string) => void;
  onRelink: (root: Source) => void;
  onError: (e: string) => void;
}

export const SettingsView = memo(function SettingsView({
  roots,
  info,
  onRescan,
  onRelink,
  onError,
}: SettingsViewProps) {
  const [pending, setPending] = useState<{root: Source; action: 'convert_source' | 'remove_source'} | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const openConfirmation = (root: Source, action: 'convert_source' | 'remove_source') => {opener.current = document.activeElement as HTMLElement;setPending({root, action});};
  const closeConfirmation = () => {setPending(null);opener.current?.focus();};
  const [pages, setPages] = useState<Record<string, number>>({});
  const [busy, setBusy] = useState(false);
  const act = async () => {
    if (!pending || busy) return;
    setBusy(true);
    try { await call(pending.action, {id: pending.root.id, confirmed: true}); closeConfirmation(); }
    catch (e) {onError(String(e));} finally {setBusy(false);}
  };
  const relinkFile = async (id: string) => {
    try {const path = await call<string | null>('choose_file');if (path) await call('relink_file', {id,path});}
    catch (e) {onError(String(e));}
  };
  return (
    <section className="settings-body">
      <h2>Sources</h2>
      {roots.length ? (
        roots.map((root) => (
          <div className="setting-row" key={root.id}>
            <div className="folder-detail">
              <strong>{root.name}</strong>
              <code>{root.root}</code>
              <small className="muted">
                {root.scope === 'files' ? 'Selected files only' : 'Recursive folder'} · {root.available ? 'Available' : 'Offline'}
              </small>
              {root.scope === 'files' && <ul aria-label={`Selected files in ${root.name}`}>
                {(root.files || []).slice((pages[root.id] || 0) * 100, ((pages[root.id] || 0) + 1) * 100).map(file => <li key={file.relative_path}>
                  <code>{file.relative_path}</code> · {file.status}
                  {file.sound_id && <button onClick={() => void relinkFile(file.sound_id!)} aria-label={`Relink ${file.relative_path}`}>Relink file</button>}
                </li>)}
                {(root.files?.length || 0) > 100 && <li>
                  <button disabled={!pages[root.id]} onClick={() => setPages(p => ({...p, [root.id]: (p[root.id] || 0) - 1}))}>Previous files</button>
                  <span>Page {(pages[root.id] || 0) + 1} of {Math.ceil((root.files?.length || 0) / 100)}</span>
                  <button disabled={((pages[root.id] || 0) + 1) * 100 >= (root.files?.length || 0)} onClick={() => setPages(p => ({...p, [root.id]: (p[root.id] || 0) + 1}))}>Next files</button>
                </li>}
              </ul>}
            </div>
            <div className="header-actions">
              <button
                className="icon-button"
                title="Rescan folder"
                aria-label={`Rescan ${root.name}`}
                onClick={() => onRescan(root.id)}
              >
                <RefreshCw size={16} aria-hidden="true" />
              </button>
              <button onClick={() => onRelink(root)}>Relink folder</button>
              {root.scope === 'files' && <button onClick={() => openConfirmation(root, 'convert_source')}>Import entire folder</button>}
              <button onClick={() => openConfirmation(root, 'remove_source')}>Remove source</button>
            </div>
          </div>
        ))
      ) : (
        <p className="muted">No sources added.</p>
      )}

      {pending && <div className="modal-backdrop"><section className="modal" role="dialog" aria-modal="true" aria-label="Confirm source change" onKeyDown={e => {
        if (e.key === 'Escape' && !busy) {e.preventDefault();closeConfirmation();}
        if (e.key === 'Tab') {
          const buttons = e.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
          const first = buttons[0], last = buttons[buttons.length - 1];
          if (e.shiftKey && document.activeElement === first) {e.preventDefault();last?.focus();}
          else if (!e.shiftKey && document.activeElement === last) {e.preventDefault();first?.focus();}
        }
      }}>
        <p>{pending.action === 'convert_source' ? `Import all supported files recursively from ${pending.root.root}? This includes siblings of your selected files.` : `Remove ${pending.root.name} from the catalog? This deletes its annotations and clip recipes. Original media stays on disk.`}</p>
        <button disabled={busy} onClick={() => void act()}>Confirm</button>
        <button autoFocus disabled={busy} onClick={closeConfirmation}>Cancel</button>
      </section></div>}
      <CatalogData onError={onError} />
      <McpSettings roots={roots} onError={onError} />
      <h2>Intelligence</h2>
      <div className="setting-row">
        <span>AI features</span>
        <span className="muted">Disabled · AI API or local model needed</span>
      </div>

      <h2>Application</h2>
      <div className="setting-row">
        <span>Version</span>
        <span>{info?.version || '0.1.0'} · Development</span>
      </div>
      <div className="setting-row">
        <span>Media engine</span>
        <span>{info?.media_tools ? 'Available' : 'Unavailable'}</span>
      </div>
      <div className="setting-row">
        <span>Data directory</span>
        <code>{info?.data_directory || 'Desktop application only'}</code>
      </div>
    </section>
  );
});
