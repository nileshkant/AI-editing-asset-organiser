import React, { memo, useState, useCallback, useMemo } from 'react';
import { Pause, Play, Square, Volume2, VolumeX } from 'lucide-react';
import { duration } from '../../api';
import type { Sound, PlaybackStatus } from '../../types';

interface TransportBarProps {
  playback: PlaybackStatus | null;
  playingSound: Sound | null;
  selected: Sound | null;
  volume: number;
  muted: boolean;
  onTogglePlay: () => void;
  onStop: () => void;
  onSeek: (pos: number) => void;
  onChangeVolume: (v: number) => void;
  onToggleMute: () => void;
}

export const TransportBar = memo(function TransportBar({
  playback,
  playingSound,
  selected,
  volume,
  muted,
  onTogglePlay,
  onStop,
  onSeek,
  onChangeVolume,
  onToggleMute,
}: TransportBarProps) {
  // Fix 3: isScrubbing guard to prevent slider jitter during drag
  const [isScrubbing, setIsScrubbing] = useState(false);
  const [scrubValue, setScrubValue] = useState(0);

  const currentDuration =
    playback?.duration_seconds ||
    playingSound?.profile?.duration ||
    selected?.profile?.duration ||
    1;

  const currentPosition = playback?.position_seconds || 0;
  const displayPosition = isScrubbing ? scrubValue : currentPosition;

  const title =
    playingSound?.title ||
    (playback?.sound_id
      ? 'Playing audio'
      : selected?.title || 'No audio selected');

  const statusText = useMemo(() => {
    if (playback?.error) return playback.error;
    switch (playback?.state) {
      case 'playing':
        return 'Playing';
      case 'paused':
        return 'Paused';
      case 'finished':
        return 'Finished';
      default:
        return 'Stopped';
    }
  }, [playback?.state, playback?.error]);

  const handleScrubStart = useCallback(() => {
    setIsScrubbing(true);
    setScrubValue(currentPosition);
  }, [currentPosition]);

  const handleScrubChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const val = Number(e.target.value);
      if (isScrubbing) {
        setScrubValue(val);
      } else {
        onSeek(val);
      }
    },
    [isScrubbing, onSeek],
  );

  const handleScrubEnd = useCallback(() => {
    if (isScrubbing) {
      onSeek(scrubValue);
      setIsScrubbing(false);
    }
  }, [isScrubbing, scrubValue, onSeek]);

  const handleVolumeChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onChangeVolume(Number(e.target.value));
    },
    [onChangeVolume],
  );

  // Compute slider fill percentage for CSS custom property
  const seekPercent =
    currentDuration > 0 ? (displayPosition / currentDuration) * 100 : 0;
  const volumePercent = (muted ? 0 : volume) * 100;

  return (
    <footer className="transport" aria-label="Audio transport">
      <div className="transport-controls">
        <button
          className="icon-button"
          title={playback?.state === 'playing' ? 'Pause' : 'Play'}
          aria-label={playback?.state === 'playing' ? 'Pause' : 'Play'}
          onClick={onTogglePlay}
        >
          {playback?.state === 'playing' ? (
            <Pause size={18} aria-hidden="true" />
          ) : (
            <Play size={18} aria-hidden="true" />
          )}
        </button>
        <button
          className="icon-button"
          title="Stop"
          aria-label="Stop"
          disabled={!playback || playback.state === 'stopped'}
          onClick={onStop}
        >
          <Square size={16} aria-hidden="true" />
        </button>
      </div>

      <div className="transport-info">
        <strong className="transport-title" title={title}>
          {title}
        </strong>
        <small className="transport-meta">
          {playback?.error ? (
            <span style={{ color: 'var(--danger)' }}>{statusText}</span>
          ) : (
            statusText
          )}
        </small>
      </div>

      <div className="transport-scrub">
        <span className="transport-time">{duration(displayPosition)}</span>
        <input
          type="range"
          className="transport-slider"
          aria-label="Seek position"
          aria-valuemin={0}
          aria-valuemax={currentDuration}
          aria-valuenow={displayPosition}
          aria-valuetext={`${duration(displayPosition)} of ${duration(currentDuration)}`}
          min="0"
          max={currentDuration}
          step="0.05"
          value={displayPosition}
          style={{ '--slider-fill': `${seekPercent}%` } as React.CSSProperties}
          onPointerDown={handleScrubStart}
          onChange={handleScrubChange}
          onPointerUp={handleScrubEnd}
        />
        <span className="transport-time">{duration(currentDuration)}</span>
      </div>

      <div
        className="transport-meter"
        title="Output Level"
        aria-label="Output Level"
        role="meter"
        aria-valuenow={Math.round((playback?.peak || 0) * 100)}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          className="transport-meter-bar"
          style={{
            width: `${Math.min(100, Math.round((playback?.peak || 0) * 100))}%`,
          }}
        />
      </div>

      <div className="transport-volume">
        <button
          className="icon-button"
          title={muted || volume === 0 ? 'Unmute' : 'Mute'}
          aria-label={muted || volume === 0 ? 'Unmute' : 'Mute'}
          onClick={onToggleMute}
        >
          {muted || volume === 0 ? (
            <VolumeX size={16} aria-hidden="true" />
          ) : (
            <Volume2 size={16} aria-hidden="true" />
          )}
        </button>
        <input
          type="range"
          className="volume-slider"
          aria-label="Volume"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(volumePercent)}
          aria-valuetext={`${Math.round(volumePercent)} percent`}
          min="0"
          max="1"
          step="0.02"
          value={muted ? 0 : volume}
          style={{ '--slider-fill': `${volumePercent}%` } as React.CSSProperties}
          onChange={handleVolumeChange}
        />
      </div>
    </footer>
  );
});
