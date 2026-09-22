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
    return Promise.reject(new Error('Open SoundShelf as a desktop application.'));
  }
  return invoke<T>(command, args);
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
