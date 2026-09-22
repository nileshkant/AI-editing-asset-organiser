import React, { memo } from 'react';
import { FolderPlus } from 'lucide-react';
import type { Page } from '../../types';

interface PageHeaderProps {
  page: Page;
  onAddFolder: () => void;
}

export const PageHeader = memo(function PageHeader({
  page,
  onAddFolder,
}: PageHeaderProps) {
  const title =
    page === 'library' ? 'Library' : page[0].toUpperCase() + page.slice(1);

  return (
    <header className="page-header">
      <div>
        <p className="eyebrow">YOUR AUDIO WORKSPACE</p>
        <h1>{title}</h1>
      </div>
      <div className="header-actions">
        <span className="build-label">Development build</span>
        <button className="primary" onClick={onAddFolder}>
          <FolderPlus size={17} aria-hidden="true" />
          Add folder
        </button>
      </div>
    </header>
  );
});
