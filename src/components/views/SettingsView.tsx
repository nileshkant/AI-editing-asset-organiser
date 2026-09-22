import React, { memo } from 'react';
import { RefreshCw } from 'lucide-react';
import { call } from '../../api';
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
  return (
    <section className="settings-body">
      <h2>Folders</h2>
      {roots.length ? (
        roots.map((root) => (
          <div className="setting-row" key={root.id}>
            <div className="folder-detail">
              <strong>{root.name}</strong>
              <code>{root.root}</code>
              <small className="muted">
                {root.available ? 'Available' : 'Offline'}
              </small>
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
              <button onClick={() => onRelink(root)}>Relink</button>
            </div>
          </div>
        ))
      ) : (
        <p className="muted">No folders added.</p>
      )}

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
