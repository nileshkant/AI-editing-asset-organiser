import React, { memo, useCallback } from 'react';
import { AudioLines, Heart, Pause, Play } from 'lucide-react';
import { duration } from '../../api';
import type { Sound, PlaybackStatus } from '../../types';

interface SoundRowProps {
  sound: Sound;
  index: number;
  isSelected: boolean;
  isFocused: boolean;
  playback: PlaybackStatus | null;
  isPlayPending: boolean;
  onSelect: (sound: Sound, index: number) => void;
  onPlay: (sound: Sound) => void;
  onPause: () => void;
  onResume: () => void;
  onFavorite: (sound: Sound) => void;
  onFocus: (index: number) => void;
}

export const SoundRow = memo(function SoundRow({
  sound,
  index,
  isSelected,
  isFocused,
  playback,
  isPlayPending,
  onSelect,
  onPlay,
  onPause,
  onResume,
  onFavorite,
  onFocus,
}: SoundRowProps) {
  const isThisPlaying =
    playback?.sound_id === sound.id && playback?.state === 'playing';
  const isThisActive = playback?.sound_id === sound.id;
  // Disable the play button while the play IPC for THIS sound is in-flight.
  const isThisPending = isPlayPending && playback?.sound_id === sound.id;

  const handleSelect = useCallback(
    () => onSelect(sound, index),
    [onSelect, sound, index],
  );

  const handlePlayToggle = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      if (isThisPending) return; // guard: ignore while pending
      if (isThisPlaying) {
        onPause();
      } else if (isThisActive && playback?.state === 'paused') {
        onResume();
      } else {
        onPlay(sound);
      }
    },
    [isThisPending, isThisPlaying, isThisActive, playback?.state, onPause, onResume, onPlay, sound],
  );

  const handleFavorite = useCallback(
    () => onFavorite(sound),
    [onFavorite, sound],
  );

  const handleFocus = useCallback(
    () => onFocus(index),
    [onFocus, index],
  );

  const tags = (
    sound.user_tags.length ? sound.user_tags : sound.profile?.tags || []
  )
    .slice(0, 3)
    .map((t) => t.replaceAll('_', ' '))
    .join(' · ');

  const className = [
    'sound-row',
    isSelected && 'selected',
    isThisPlaying && 'playing',
    isFocused && 'keyboard-focused',
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div
      role="option"
      aria-selected={isSelected}
      aria-current={isThisPlaying ? 'true' : undefined}
      tabIndex={0}
      onFocus={handleFocus}
      className={className}
    >
      <button
        className="sound-select"
        onClick={handleSelect}
        aria-label={`${sound.title}, duration ${duration(sound.profile?.duration || 0)}`}
      >
        <span className="file-mark" aria-hidden="true">
          <AudioLines size={20} />
        </span>
        <span className="sound-label">
          <strong>
            {isThisPlaying && (
              <span
                className="playing-dot"
                title="Currently playing"
                aria-hidden="true"
              />
            )}
            {sound.title}
          </strong>
          <small>{tags}</small>
        </span>
        <span className="sound-duration">
          {duration(sound.profile?.duration || 0)}
        </span>
      </button>

      <button
        className="icon-button"
        title={isThisPlaying ? `Pause ${sound.title}` : `Play ${sound.title}`}
        aria-label={
          isThisPending
            ? `Loading ${sound.title}`
            : isThisPlaying ? `Pause ${sound.title}` : `Play ${sound.title}`
        }
        disabled={isThisPending}
        onClick={handlePlayToggle}
      >
        {isThisPlaying ? (
          <Pause size={15} aria-hidden="true" />
        ) : (
          <Play size={15} aria-hidden="true" />
        )}
      </button>

      <button
        className="icon-button favorite"
        title={sound.favorite ? 'Remove favorite' : 'Add favorite'}
        aria-label={`${sound.favorite ? 'Unfavorite' : 'Favorite'} ${sound.title}`}
        aria-pressed={sound.favorite}
        onClick={handleFavorite}
      >
        <Heart
          size={16}
          fill={sound.favorite ? 'currentColor' : 'none'}
          aria-hidden="true"
        />
      </button>
    </div>
  );
});
