import { invoke, isTauri } from '@tauri-apps/api/core';

export * from './types';

/**
 * Invokes a Tauri command with the given arguments.
 * Rejects if the application is not running in the Tauri desktop environment.
 * 
 * @param command - The name of the Tauri command to call.
 * @param args - Optional arguments to pass to the command.
 * @returns A promise resolving to the expected return type `T`.
 */
export function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    return Promise.reject(new Error('Open CreativeShelf as a desktop application.'));
  }
  return Promise.resolve(invoke<T>(command, args));
}

/**
 * Formats a duration in seconds into a human-readable time string (e.g., 1:23.45).
 * Supports negative and NaN inputs, as well as durations over an hour.
 * 
 * @param seconds - The duration in seconds.
 * @returns The formatted duration string.
 */
export function duration(seconds: number): string {
  if (Number.isNaN(seconds)) return '0:00.00';
  if (seconds < 0) seconds = 0;

  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainingSeconds = seconds % 60;
  
  const secondsStr = remainingSeconds.toFixed(2).padStart(5, '0');
  
  if (hours > 0) {
    const minutesStr = minutes.toString().padStart(2, '0');
    return `${hours}:${minutesStr}:${secondsStr}`;
  }
  
  return `${minutes}:${secondsStr}`;
}

/**
 * Converts a time in seconds to an exact source sample frame counter as a decimal string.
 * Uses round-to-nearest sample frame and clamps non-negative.
 */
export function secondsToFrame(seconds: number, sampleRate: number): string {
  if (Number.isNaN(seconds) || seconds < 0 || sampleRate <= 0) return '0';
  return Math.max(0, Math.round(seconds * sampleRate)).toString();
}

/**
 * Converts a source sample frame counter to seconds given a sample rate.
 */
export function frameToSeconds(frame: string | number, sampleRate: number): number {
  if (sampleRate <= 0) return 0;
  const num = typeof frame === 'number' ? frame : parseInt(frame, 10);
  if (Number.isNaN(num) || num < 0) return 0;
  return num / sampleRate;
}

/**
 * Creates a new virtual clip recipe bound to source sample frames.
 */
export function createClip(
  soundId: string,
  name: string,
  recipe: import('./types').ClipRecipe,
): Promise<import('./types').Clip> {
  return call('create_clip', { soundId, name, recipe });
}

/**
 * Retrieves a clip by ID with staleness evaluation.
 */
export function getClip(id: string): Promise<import('./types').Clip> {
  return call('get_clip', { id });
}

/**
 * Lists all saved clip variants for a given sound.
 */
export function listClips(soundId: string): Promise<import('./types').Clip[]> {
  return call('list_clips', { soundId });
}

/**
 * Updates a clip's name and recipe with revision concurrency checking.
 */
export function updateClip(
  id: string,
  name: string,
  recipe: import('./types').ClipRecipe,
  expectedRevision: number,
): Promise<import('./types').Clip> {
  return call('update_clip', { id, name, recipe, expectedRevision });
}

/**
 * Rebinds a stale clip to the current version of its source audio file.
 */
export function rebindClip(id: string): Promise<import('./types').Clip> {
  return call('rebind_clip', { id });
}

/**
 * Deletes a clip and its revision history.
 */
export function deleteClip(id: string): Promise<void> {
  return call('delete_clip', { id });
}

/**
 * Plays the clip region of a sound (start_frame → end_frame) using ffmpeg -ss/-t.
 * The backend resolves clip boundaries from the DB and calls Player::play_clip().
 */
export function playClip(soundId: string, clipId: string): Promise<void> {
  return call('playback_play_clip', { id: soundId, clipId });
}

export function chooseExportDestination(format: 'wav' | 'flac'): Promise<import('./types').DestinationGrant | null> {
  return call('choose_export_destination', { format });
}
export function exportClip(destinationId: string, clipId: string, expectedRevision: number, options: import('./types').ExportOptions): Promise<import('./types').ExportResult> {
  return call('export_clip', { destinationId, clipId, expectedRevision, options });
}
export function cancelExport(destinationId: string): Promise<void> {
  return call('cancel_export', { destinationId });
}

// Opaque preview IDs authorize only selections approved by native OS adapters.
export const imageImportCapabilities = () => call<import('./types').ImageImportCapabilities>('image_import_capabilities');
export const chooseMediaImport = (folders: boolean) => call<string[]>('choose_media_import', { folders });
export const previewMediaImport = (requestId: string, paths: string[], mode: import('./types').ImageImportMode) => call<import('./types').MediaImportPreview>('preview_media_import', { requestId, paths, mode });
export const cancelMediaPreview = (requestId: string, previewId: string | null) => call<void>('cancel_media_preview', { requestId, previewId });
export const confirmMediaImport = (previewId: string) => call<import('./types').ImageImportJob[]>('confirm_media_import', { previewId });
export const imageImportJobs = (after: string | null = null) => call<import('./types').ImageImportJob[]>('image_import_jobs', { after });
export const imageImportItems = (id: string, after: string | null = null) => call<import('./types').ImageImportItemPage>('image_import_items', { id, after });
export const cancelImageImport = (job: import('./types').ImageImportJob) => call<import('./types').ImageImportJob>('cancel_image_import', { id: job.id, sourceGeneration: job.source_generation });
export const retryImageImport = (job: import('./types').ImageImportJob) => call<import('./types').ImageImportJob>('retry_image_import', { id: job.id, sourceGeneration: job.source_generation });
export const importedImages = (sourceId: string, after: string | null = null) => call<import('./types').ImportedImagePage>('imported_images', { sourceId, after });

// SS-047: refresh, opt-in watching, relink (native picker only) and explicit moves.
export const imageSourceStatuses = () => call<import('./types').ImageSourceStatus[]>('image_source_statuses');
export const checkImageSource = (id: string) => call<import('./types').ImageSourceStatus>('check_image_source', { id });
export const refreshImageSource = (id: string, scan: 'discover' | 'listed', verify: boolean) =>
  call<import('./types').ImageImportJob>('refresh_image_source', { id, scan, verify });
export const setImageSourceWatch = (id: string, enabled: boolean) => call<import('./types').ImageSourceStatus>('set_image_source_watch', { id, enabled });
export const chooseImageRelink = (id: string) => call<import('./types').ImageRelinkPreview | null>('choose_image_relink', { id });
export const confirmImageRelink = (previewId: string, acknowledged: boolean) =>
  call<import('./types').ImageImportJob>('confirm_image_relink', { previewId, acknowledged });
export const cancelImageRelink = (previewId: string) => call<void>('cancel_image_relink', { previewId });
export const imageMoveCandidates = (sourceId: string) => call<import('./types').ImageMoveCandidate[]>('image_move_candidates', { sourceId });
export const reconcileImageMove = (c: import('./types').ImageMoveCandidate) =>
  call<unknown>('reconcile_image_move', { missingId: c.missing_id, presentId: c.present_id, missingRevision: c.missing_revision, presentRevision: c.present_revision });

type PreviewTarget = { assetId: string; version: string; tier: import('./types').ImagePreviewTier; tile: [number, number] | null };
/**
 * Parses the SS-044 binary preview envelope (`CSPV`, u32 LE metadata length,
 * metadata JSON, encoded image) and rejects a reply for any other asset/version/tier.
 */
export function parsePreviewEnvelope(raw: unknown, expected: PreviewTarget): { meta: import('./types').ImagePreviewMeta; blob: Blob } {
  const bytes = raw instanceof ArrayBuffer ? new Uint8Array(raw) : raw instanceof Uint8Array ? raw : Array.isArray(raw) ? Uint8Array.from(raw as number[]) : null;
  if (!bytes || bytes.length < 9 || String.fromCharCode(...bytes.subarray(0, 4)) !== 'CSPV') throw new Error('INVALID_DATA');
  const length = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(4, true);
  if (length > 16384 || 8 + length >= bytes.length) throw new Error('INVALID_DATA');
  let meta: import('./types').ImagePreviewMeta;
  try { meta = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + length))); } catch { throw new Error('INVALID_DATA'); }
  const dimension = (n: unknown) => Number.isInteger(n) && (n as number) >= 1 && (n as number) <= 32768;
  const tile = expected.tile ? `${expected.tile[0]},${expected.tile[1]}` : null;
  if (!meta || meta.asset_id !== expected.assetId || meta.version !== expected.version || meta.tier !== expected.tier ||
    (meta.tile ? `${meta.tile[0]},${meta.tile[1]}` : null) !== tile || !['image/png', 'image/jpeg'].includes(meta.mime) ||
    ![meta.width, meta.height, meta.source_width, meta.source_height].every(dimension) ||
    typeof meta.original_available !== 'boolean' || meta.offline_reference !== !meta.original_available) throw new Error('INVALID_DATA');
  return { meta, blob: new Blob([bytes.slice(8 + length)], { type: meta.mime }) };
}
/** Requests a preview by asset ID and content version only; no paths cross IPC. */
export async function imagePreview(requestId: string, assetId: string, version: string, tier: import('./types').ImagePreviewTier, tile: [number, number] | null = null) {
  const raw = await call<unknown>('image_preview', { requestId, assetId, version, tier, tile });
  return parsePreviewEnvelope(raw, { assetId, version, tier, tile });
}
export const cancelImagePreview = (requestId: string) => call<boolean>('cancel_image_preview', { requestId });
/** Gallery window; `sourceId` null means every registered source. */
export const imageGalleryPage = (sourceId: string | null, sort: import('./types').GallerySort, offset: number, limit: number) =>
  call<import('./types').GalleryPage>('image_gallery_page', { sourceId, sort, offset, limit });
export const imageDetails = (id: string) => call<import('./types').ImageDetails>('image_details', { id });
/** SS-046: one image, from the revision the editor last read (CONFLICT otherwise). */
export const editImageAnnotations = (id: string, expectedRevision: number, patch: import('./types').AnnotationPatch) =>
  call<import('./types').AnnotationEditResult>('edit_image_annotations', { id, expectedRevision, patch });
export const previewImageBulkAnnotations = (ids: string[], patch: import('./types').AnnotationPatch) =>
  call<import('./types').BulkPreview>('preview_image_bulk_annotations', { ids, patch });
export const applyImageBulkAnnotations = (targets: import('./types').BulkTarget[], patch: import('./types').AnnotationPatch) =>
  call<import('./types').BulkResult>('apply_image_bulk_annotations', { targets, patch });
export const imageAnnotationHistory = (limit = 100) => call<import('./types').AnnotationAction[]>('image_annotation_history', { limit });
export const undoImageAnnotationAction = (actionId: string) => call<import('./types').BulkResult>('undo_image_annotation_action', { actionId });
export const imageTags = (sourceId: string | null) => call<import('./types').TagList>('image_tags', { sourceId });
export const previewImageTagRename = (from: string, to: string, sourceId: string | null) =>
  call<import('./types').RenamePreview>('preview_image_tag_rename', { from, to, sourceId });
export const renameImageTag = (from: string, to: string, sourceId: string | null, keepAlias: boolean) =>
  call<import('./types').BulkResult>('rename_image_tag', { from, to, sourceId, keepAlias });
export const removeImageTagAlias = (alias: string) => call<null>('remove_image_tag_alias', { alias });
