import { useCallback, useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import * as api from '../api';
import type { ImageImportJob, ImageSourceStatus } from '../types';

/**
 * SS-047 source statuses: loaded once, re-read whenever a job changes, and polled
 * slowly only while the panel is visible (status queries count every image).
 */
export function useImageSources(enabled: boolean, jobs: ImageImportJob[], onError: (message: string) => void, visible = true) {
  const [sources, setSources] = useState<ImageSourceStatus[]>([]);
  const alive = useRef(true), turn = useRef(0);
  const refresh = useCallback(async () => {
    if (!isTauri()) return;
    const current = ++turn.current;
    try {
      const next = await api.imageSourceStatuses();
      if (alive.current && current === turn.current && Array.isArray(next)) setSources(next);
    } catch { /* keep the last known statuses; actions report their own errors */ }
  }, []);
  const revision = jobs.map(j => `${j.id}:${j.sequence}`).join(',');
  useEffect(() => { void refresh(); }, [refresh, revision, enabled]);
  useEffect(() => {
    alive.current = true;
    const timer = visible ? setInterval(() => void refresh(), 5000) : undefined;
    return () => { alive.current = false; clearInterval(timer); };
  }, [refresh, visible]);
  /** Run one action, then re-read statuses; errors go to the shared error banner. */
  const act = useCallback(async <T,>(task: () => Promise<T>): Promise<T | undefined> => {
    try { return await task(); } catch (e) { onError(String(e)); return undefined; } finally { void refresh(); }
  }, [onError, refresh]);
  return { sources, refresh, act };
}
export type ImageSourcesController = ReturnType<typeof useImageSources>;
