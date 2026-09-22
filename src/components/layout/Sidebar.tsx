import React, { memo } from 'react';
import {
  AudioLines,
  FolderOpen,
  FolderPlus,
  Heart,
  Inbox,
  Library,
  Settings2,
  ShieldCheck,
} from 'lucide-react';
import type { Page, Source, Progress } from '../../types';

interface SidebarProps {
  page: Page;
  setPage: (page: Page) => void;
  roots: Source[];
  source: string;
  setSource: (id: string) => void;
  activeJobCount: number;
  onAddFolder: () => void;
}

const NAV_ITEMS = [
  { id: 'library' as const, label: 'Library', icon: Library },
  { id: 'favorites' as const, label: 'Favorites', icon: Heart },
  { id: 'imports' as const, label: 'Imports', icon: Inbox },
  { id: 'settings' as const, label: 'Settings', icon: Settings2 },
];

export const Sidebar = memo(function Sidebar({
  page,
  setPage,
  roots,
  source,
  setSource,
  activeJobCount,
  onAddFolder,
}: SidebarProps) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <AudioLines size={25} aria-hidden="true" />
        <strong>SoundShelf</strong>
      </div>

      <nav aria-label="Main navigation">
        {NAV_ITEMS.map((n) => (
          <button
            key={n.id}
            className={`nav ${page === n.id ? 'active' : ''}`}
            aria-current={page === n.id ? 'page' : undefined}
            onClick={() => setPage(n.id)}
          >
            <n.icon size={18} aria-hidden="true" />
            <span>{n.label}</span>
            {n.id === 'imports' && activeJobCount > 0 && (
              <span className="count">{activeJobCount}</span>
            )}
          </button>
        ))}
      </nav>

      <div className="source-heading">
        FOLDERS
        <button
          className="icon-button"
          title="Add folder"
          aria-label="Add folder"
          onClick={onAddFolder}
        >
          <FolderPlus size={15} aria-hidden="true" />
        </button>
      </div>

      <div className="source-links">
        {roots.map((root) => (
          <button
            key={root.id}
            className={`source-link ${source === root.id ? 'chosen' : ''}`}
            title={root.root}
            onClick={() => {
              setSource(source === root.id ? '' : root.id);
              setPage('library');
            }}
          >
            <FolderOpen size={15} aria-hidden="true" />
            <span>{root.name}</span>
            {!root.available && (
              <span
                className="offline-dot"
                title="Folder offline"
                aria-label="Folder offline"
              />
            )}
          </button>
        ))}
      </div>

      <div className="sidebar-bottom">
        <ShieldCheck size={15} aria-hidden="true" />
        <span>Local workspace</span>
        <span className="status-dot" aria-hidden="true" />
      </div>
    </aside>
  );
});
