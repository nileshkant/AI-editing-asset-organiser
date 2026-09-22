import React, { memo, useState } from 'react';
import { FolderOpen } from 'lucide-react';
import { call } from '../../api';
import type { Source } from '../../types';

interface RelinkModalProps {
  source: Source;
  onClose: () => void;
  onRelinked: () => void;
  onError: (e: string) => void;
}

export const RelinkModal = memo(function RelinkModal({
  source,
  onClose,
  onRelinked,
  onError,
}: RelinkModalProps) {
  const [path, setPath] = useState(source.root);

  const handleBrowse = async () => {
    try {
      const p = await call<string | null>('choose_folder');
      if (p) setPath(p);
    } catch (e) {
      onError(String(e));
    }
  };

  const handleRelink = async () => {
    try {
      await call('relink_source', { id: source.id, path });
      onRelinked();
    } catch (e) {
      onError(String(e));
    }
  };

  return (
    <div className="modal-backdrop">
      <section
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="relink-title"
      >
        <h2 id="relink-title">Relink {source.name}</h2>
        <label>
          Folder path
          <input
            autoFocus
            value={path}
            onChange={(e) => setPath(e.target.value)}
          />
        </label>
        <div className="modal-actions">
          <button onClick={onClose}>Cancel</button>
          <button onClick={handleBrowse}>
            <FolderOpen size={16} aria-hidden="true" />
            Browse
          </button>
          <button className="primary" onClick={handleRelink}>
            Verify & relink
          </button>
        </div>
      </section>
    </div>
  );
});
