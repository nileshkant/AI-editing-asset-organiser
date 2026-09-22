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
