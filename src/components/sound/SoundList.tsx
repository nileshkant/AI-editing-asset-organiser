import React, { memo, useCallback } from 'react';
import { FolderOpen, FolderPlus } from 'lucide-react';
import { SoundRow } from './SoundRow';
import type { Sound, PlaybackStatus } from '../../types';

interface SoundListProps {
  items: Sound[];
  playback: PlaybackStatus | null;
  selectedId: string | null;
  focusedIndex: number;
  onSelect: (sound: Sound, index: number) => void;
  onPlay: (sound: Sound) => void;
  onPause: () => void;
  onResume: () => void;
  onFavorite: (sound: Sound) => void;
  onFocusIndex: (index: number) => void;
  onAddFolder: () => void;
  hasSearchOrFilter: boolean;
  isFavoritesPage: boolean;
}

export const SoundList = memo(function SoundList({
  items,
  playback,
  selectedId,
  focusedIndex,
  onSelect,
  onPlay,
  onPause,
  onResume,
  onFavorite,
  onFocusIndex,
  onAddFolder,
  hasSearchOrFilter,
  isFavoritesPage,
}: SoundListProps) {
  if (!items.length) {
    return (
      <div className="empty">
        <FolderOpen size={48} strokeWidth={1} aria-hidden="true" />
        <h2>
          {hasSearchOrFilter || isFavoritesPage
            ? 'No matching sounds'
            : 'No sounds yet'}
        </h2>
        <button onClick={onAddFolder}>
          <FolderPlus size={16} aria-hidden="true" />
          Add folder
        </button>
      </div>
    );
  }

  return (
    <>
      {items.map((sound, index) => (
        <SoundRow
          key={sound.id}
          sound={sound}
          index={index}
          isSelected={selectedId === sound.id}
          isFocused={focusedIndex === index}
          playback={playback}
          onSelect={onSelect}
          onPlay={onPlay}
          onPause={onPause}
          onResume={onResume}
          onFavorite={onFavorite}
          onFocus={onFocusIndex}
        />
      ))}
    </>
  );
});
