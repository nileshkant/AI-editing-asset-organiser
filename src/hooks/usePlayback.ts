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

  const stateRef = useRef({ playingSound, selected, resultsItems });
  useEffect(() => {
    stateRef.current = { playingSound, selected, resultsItems };
  }, [playingSound, selected, resultsItems]);

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
      setPlayingSound(sound);
      setAnnouncement(`Playing ${sound.title}`);
      
      setPlayback(prev => ({
        sound_id: sound.id,
        state: 'playing' as const,
        position_seconds: 0,
        duration_seconds: sound.profile?.duration || 0,
        volume: prev?.volume ?? 1,
        peak: 0,
        error: null,
      }));
      
      await call("playback_play", { id: sound.id });
    }), []);

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
        setAnnouncement("Playback paused");
        await call("playback_pause");
      } else if (currentState === "paused") {
        setAnnouncement("Playback resumed");
        await call("playback_resume");
      }
    }), [playSound]);

  const stopPlayback = useCallback(() =>
    guard(async () => {
      setAnnouncement("Playback stopped");
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

  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    const interval = setInterval(async () => {
      try {
        const s = await call<PlaybackStatus>("playback_status");
        if (alive) setPlayback(s);
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
    playSound,
    togglePlay,
    stopPlayback,
    seekPlayback,
    changeVolume,
    toggleMute
  };
}
