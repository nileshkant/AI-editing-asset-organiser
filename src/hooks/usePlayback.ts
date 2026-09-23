import { useState, useEffect, useRef, useCallback } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { call, PlaybackStatus, Sound } from '../api';

export function usePlayback(selected: Sound | null, resultsItems: Sound[]) {
  const [playback, setPlayback] = useState<PlaybackStatus | null>(null);
  const [playingSound, setPlayingSound] = useState<Sound | null>(null);
  const [volume, setVolumeState] = useState(1.0);
  const [muted, setMuted] = useState(false);
  const [announcement, setAnnouncement] = useState("");
  const [error, setError] = useState("");
  // True while a play IPC call is in-flight. Used to prevent rapid multi-clicks from stacking.
  const [isPlayPending, setIsPlayPending] = useState(false);

  const stateRef = useRef({ playingSound, selected, resultsItems });
  useEffect(() => {
    stateRef.current = { playingSound, selected, resultsItems };
  }, [playingSound, selected, resultsItems]);

  // Tracks the active play request token to guard against re-entry.
  const playToken = useRef(0);

  const guard = async <T,>(task: () => Promise<T>): Promise<T | undefined> => {
    try {
      setError("");
      return await task();
    } catch (e) {
      setError(String(e));
      return undefined;
    }
  };

  const playSound = useCallback((sound: Sound) =>
    guard(async () => {
      // Debounce guard: if a play is already in-flight, ignore re-entry.
      if (isPlayPending) return;
      const token = ++playToken.current;
      setIsPlayPending(true);

      // Optimistic UI update: show immediately, don't wait for IPC.
      setPlayingSound(sound);
      setAnnouncement(`Playing ${sound.title}`);
      setPlayback(prev => ({
        sound_id: sound.id,
        state: 'playing' as const,
        position_seconds: 0,
        duration_seconds: sound.profile?.duration || prev?.duration_seconds || 0,
        volume: prev?.volume ?? 1,
        peak: 0,
        error: null,
      }));

      try {
        await call("playback_play", { id: sound.id });
      } finally {
        // Only clear pending if this was still the current token (not superseded).
        if (playToken.current === token) {
          setIsPlayPending(false);
        }
      }
    }), [isPlayPending]);

  const togglePlay = useCallback(() =>
    guard(async () => {
      const { playingSound, selected, resultsItems } = stateRef.current;
      
      let currentState = "stopped";
      setPlayback(prev => {
        currentState = prev?.state || "stopped";
        return prev;
      });

      if (
        !currentState ||
        currentState === "stopped" ||
        currentState === "finished"
      ) {
        if (playingSound) await playSound(playingSound);
        else if (selected) await playSound(selected);
        else if (resultsItems.length > 0) await playSound(resultsItems[0]);
      } else if (currentState === "playing") {
        // Optimistic: update state immediately before IPC confirms.
        setAnnouncement("Playback paused");
        setPlayback(prev => prev ? { ...prev, state: 'paused' as const } : prev);
        await call("playback_pause");
      } else if (currentState === "paused") {
        // Optimistic: update state immediately before IPC confirms.
        setAnnouncement("Playback resumed");
        setPlayback(prev => prev ? { ...prev, state: 'playing' as const } : prev);
        await call("playback_resume");
      }
    }), [playSound]);

  const stopPlayback = useCallback(() =>
    guard(async () => {
      setAnnouncement("Playback stopped");
      // Optimistic stop: clear state before IPC round-trip.
      setPlayback(prev => prev ? { ...prev, state: 'stopped' as const, position_seconds: 0 } : prev);
      setIsPlayPending(false);
      await call("playback_stop");
    }), []);

  const seekPlayback = useCallback((pos: number) =>
    guard(async () => {
      await call("playback_seek", { positionSeconds: pos });
    }), []);

  const changeVolume = useCallback((v: number) =>
    guard(async () => {
      setVolumeState(v);
      setMuted(v === 0);
      await call("playback_set_volume", { volume: v });
    }), []);

  const toggleMute = useCallback(() =>
    guard(async () => {
      const next = !muted;
      setMuted(next);
      setAnnouncement(next ? "Muted" : "Unmuted");
      await call("playback_set_volume", { volume: next ? 0 : volume });
    }), [muted, volume]);

  // Poll the real backend status at 120ms to keep position/peak accurate.
  // Optimistic state above handles the instant feedback; this reconciles with ground truth.
  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    const interval = setInterval(async () => {
      try {
        const s = await call<PlaybackStatus>("playback_status");
        if (alive) {
          setPlayback(s);
          // Clear pending flag once backend confirms playing or stopped.
          if (s.state === 'playing' || s.state === 'stopped' || s.state === 'finished') {
            setIsPlayPending(false);
          }
        }
      } catch (_) {}
    }, 120);
    return () => {
      alive = false;
      clearInterval(interval);
    };
  }, []);

  useEffect(() => {
    if (!playback?.sound_id) return;
    if (playingSound?.id === playback.sound_id) return;
    const inList = resultsItems.find((s) => s.id === playback.sound_id);
    if (inList) {
      setPlayingSound(inList);
    } else if (selected?.id === playback.sound_id) {
      setPlayingSound(selected);
    } else if (isTauri()) {
      call<Sound>("get_sound", { id: playback.sound_id })
        .then((s) => {
          if (s) setPlayingSound(s);
        })
        .catch(() => {});
    }
  }, [playback?.sound_id, resultsItems, selected, playingSound]);

  return {
    playback,
    playingSound,
    setPlayingSound,
    volume,
    muted,
    announcement,
    setAnnouncement,
    error,
    setError,
    isPlayPending,
    playSound,
    togglePlay,
    stopPlayback,
    seekPlayback,
    changeVolume,
    toggleMute
  };
}
