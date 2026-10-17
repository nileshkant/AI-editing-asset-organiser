import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { isTauri } from '@tauri-apps/api/core';
import * as api from '../api';
import type { ImageImportJob, ImageImportMode, MediaImportPreview } from '../types';

export function validImageJob(job: ImageImportJob): boolean {
  return !!job && typeof job.id === 'string' && typeof job.source_id === 'string' &&
    typeof job.sequence === 'string' && /^[1-9]\d{0,15}$/.test(job.sequence) && BigInt(job.sequence) <= BigInt(Number.MAX_SAFE_INTEGER) &&
    typeof job.source_generation === 'string' && /^\d{1,19}$/.test(job.source_generation);
}
export function mergeImageJob(current: ImageImportJob[], job: ImageImportJob): ImageImportJob[] {
  if (!validImageJob(job)) return current;
  const generation = BigInt(job.source_generation);
  if (current.some(j => j.source_id === job.source_id && BigInt(j.source_generation) > generation)) return current;
  const previous = current.find(j => j.id === job.id);
  if (previous && (previous.source_id !== job.source_id || previous.source_generation !== job.source_generation || BigInt(previous.sequence) >= BigInt(job.sequence))) return current;
  return [job, ...current.filter(j => j.id !== job.id)].slice(0, 1000);
}
export const importTerminal = (job: ImageImportJob) => ['complete', 'failed', 'cancelled'].includes(job.state);

export function useImageImports(onError: (message: string) => void, onOpen: () => void) {
  const [enabled, setEnabled] = useState(false);
  const [available, setAvailable] = useState(false);
  const [jobs, setJobs] = useState<ImageImportJob[]>([]);
  const [mode, setMode] = useState<ImageImportMode>('mixed');
  const [preview, setPreview] = useState<MediaImportPreview | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [previewError, setPreviewError] = useState('');
  const returnFocus = useRef<HTMLElement | null>(null);
  const databaseEpoch = useRef(0);
  const alive = useRef(true), epoch = useRef(0), request = useRef<string | null>(null), currentPreview = useRef<string | null>(null), pending = useRef(false);
  const refresh = useCallback(async () => {
    const revision = databaseEpoch.current;
    const all: ImageImportJob[] = []; let after: string | null = null;
    for (let page = 0; page < 20; ++page) {
      const batch = await api.imageImportJobs(after);
      if (!Array.isArray(batch) || !alive.current || revision !== databaseEpoch.current) return;
      all.push(...batch.filter(validImageJob));
      if (batch.length < 50) break;
      const next = batch.at(-1)?.id; if (!next || next === after) break; after = next;
    }
    if (alive.current && revision === databaseEpoch.current) setJobs(old => all.reduce(mergeImageJob, old));
  }, []);
  useEffect(() => {
    alive.current = true;
    if (!isTauri()) { setAvailable(true); return () => { alive.current = false; }; }
    let disposed = false, busy = false;
    void api.imageImportCapabilities().then(cap => { if (!disposed) { setEnabled(!!cap?.enabled); setAvailable(true); } }).catch(() => { if (!disposed) setAvailable(true); });
    const poll = () => { if (busy) return; busy = true; void refresh().catch(() => {}).finally(() => { busy = false; }); };
    poll(); const timer = setInterval(poll, 1000);
    const subscription = listen<ImageImportJob>('image-import-progress', event => { if (!disposed && validImageJob(event.payload)) void refresh().catch(() => {}); }).catch(() => undefined);
    const reset = listen('image-import-reset', () => { if (!disposed) { ++databaseEpoch.current; ++epoch.current; setJobs([]); setPreview(null); setPreviewError(''); setPreparing(false); pending.current = false; request.current = null; currentPreview.current = null; void refresh().catch(() => {}); } }).catch(() => undefined);
    return () => { disposed = true; alive.current = false; ++epoch.current; clearInterval(timer); void subscription.then(fn => fn?.()); void reset.then(fn => fn?.()); if (request.current) void api.cancelMediaPreview(request.current, currentPreview.current).catch(() => {}); };
  }, [refresh]);
  const prepare = useCallback(async (paths: string[]) => {
    if (!enabled || !paths.length || pending.current) return;
    returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    pending.current = true; const turn = ++epoch.current; const id = crypto.randomUUID(); request.current = id;
    currentPreview.current = null; setPreview(null); setPreparing(true); setPreviewError(''); onOpen();
    try {
      const result = await api.previewMediaImport(id, paths, mode);
      if (!alive.current || turn !== epoch.current) { void api.cancelMediaPreview(id, result.id).catch(() => {}); return; }
      currentPreview.current = result.id; setPreview(result);
    } catch (e) { if (alive.current && turn === epoch.current) setPreviewError(String(e)); }
    finally { if (alive.current && turn === epoch.current) { setPreparing(false); pending.current = false; } }
  }, [enabled, mode, onOpen]);
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    const drop = listen<string[]>('image-import-drop', event => { if (!disposed) void prepare(event.payload); }).catch(() => undefined);
    const error = listen<string>('image-import-drop-error', event => { if (!disposed) onError(String(event.payload)); }).catch(() => undefined);
    return () => { disposed = true; void drop.then(fn => fn?.()); void error.then(fn => fn?.()); };
  }, [enabled, prepare, onError]);
  const choose = useCallback(async (folders: boolean) => {
    if (!enabled || pending.current) return;
    try { const paths = await api.chooseMediaImport(folders); if (alive.current) await prepare(paths); } catch (e) { if (alive.current) onError(String(e)); }
  }, [enabled, prepare, onError]);
  const close = useCallback(() => {
    if (confirming) return;
    ++epoch.current; pending.current = false; setPreview(null); setPreparing(false); setPreviewError('');
    if (request.current) void api.cancelMediaPreview(request.current, currentPreview.current).catch(onError);
    request.current = null; currentPreview.current = null;
  }, [confirming, onError]);
  const confirm = useCallback(async () => {
    if (!preview || pending.current) return;
    pending.current = true; setConfirming(true); setPreviewError('');
    try {
      const created = await api.confirmMediaImport(preview.id);
      if (alive.current) { setJobs(old => created.reduce(mergeImageJob, old)); setPreview(null); request.current = null; currentPreview.current = null; }
    } catch (e) { if (alive.current) setPreviewError(String(e)); }
    finally { pending.current = false; if (alive.current) setConfirming(false); }
  }, [preview]);
  const action = useCallback(async (job: ImageImportJob, retry: boolean) => {
    try { const result = await (retry ? api.retryImageImport(job) : api.cancelImageImport(job)); if (alive.current) setJobs(old => mergeImageJob(old, result)); } catch (e) { if (alive.current) onError(String(e)); }
  }, [onError]);
  return { returnFocus, enabled, available, jobs, mode, setMode, preview, preparing, confirming, previewError, choose, close, confirm, action };
}
export type ImageImportsController = ReturnType<typeof useImageImports>;
