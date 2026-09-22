import React, { memo } from 'react';
import { FolderOpen, Inbox, RefreshCw } from 'lucide-react';
import type { Progress, Source } from '../../types';

interface ImportsViewProps {
  jobs: Progress[];
  roots: Source[];
  onCancelImport: () => void;
}

export const ImportsView = memo(function ImportsView({
  jobs,
  roots,
  onCancelImport,
}: ImportsViewProps) {
  if (!jobs.length) {
    return (
      <section className="settings-body">
        <div className="empty">
          <Inbox size={40} aria-hidden="true" />
          <h2>No imports</h2>
        </div>
      </section>
    );
  }

  return (
    <section className="settings-body">
      {jobs.map((job) => (
        <div className="job-row" key={job.source_id}>
          <div className="job-heading">
            <strong>
              {roots.find((r) => r.id === job.source_id)?.name || 'Folder'}
            </strong>
            <span>{job.status}</span>
          </div>
          <progress value={job.completed} max={Math.max(1, job.total)} />
          <p>
            {job.completed} / {job.total} files · {job.reused} reused ·{' '}
            {job.failed} failed
          </p>
          <small className="muted path-text">{job.current}</small>
          {job.errors.length > 0 && (
            <details>
              <summary>Errors ({job.errors.length})</summary>
              {job.errors.map((e, i) => (
                <p key={i} className="path-text">
                  {e}
                </p>
              ))}
            </details>
          )}
          {job.status === 'analyzing' && (
            <button onClick={onCancelImport}>Cancel import</button>
          )}
        </div>
      ))}
    </section>
  );
});
