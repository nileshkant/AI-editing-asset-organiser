import { useEffect, useRef } from 'react';
import { PlaybackStatus, SearchResults, Sound, Source } from '../api';

export interface KeyboardShortcutsConfig {
  playback: PlaybackStatus | null;
  selected: Sound | null;
  playingSound: Sound | null;
  results: SearchResults;
  focusedIndex: number;
  offset: number;
  relink: Source | null;
  error: string;
  filters: boolean;
  
  setRelink: (s: Source | null) => void;
  setSelected: (s: Sound | null) => void;
  setError: (e: string) => void;
  setFilters: (f: boolean) => void;
  togglePlay: () => void;
  searchInputRef: React.RefObject<HTMLInputElement | null>;
  toggleMute: () => void;
  setFocusedIndex: (i: number) => void;
  select: (s: Sound, i: number) => void;
  setOffset: (o: number) => void;
  setAnnouncement: (a: string) => void;
}

export function useKeyboardShortcuts(config: KeyboardShortcutsConfig) {
  const stateRef = useRef(config);
  
  useEffect(() => {
    stateRef.current = config;
  });
  
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const s = stateRef.current;
      const tag = (e.target as HTMLElement)?.tagName?.toLowerCase();
      const isInput =
        tag === "input" ||
        tag === "textarea" ||
        tag === "select" ||
        (e.target as HTMLElement)?.isContentEditable;
        
      if (e.key === "Escape") {
        if (s.relink) {
          s.setRelink(null);
          return;
        }
        if (s.selected) {
          s.setSelected(null);
          s.setAnnouncement("Details closed");
          return;
        }
        if (s.error) {
          s.setError("");
          return;
        }
        if (s.filters) {
          s.setFilters(false);
          return;
        }
        return;
      }
      if (isInput) return;
      if (e.code === "Space") {
        e.preventDefault();
        void s.togglePlay();
        return;
      }
      if (e.key === "/" && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        s.searchInputRef.current?.focus();
        return;
      }
      if (e.key === "m" || e.key === "M") {
        e.preventDefault();
        void s.toggleMute();
        return;
      }
      if (e.key === "ArrowDown" || e.key === "j") {
        if (!s.results.items.length) return;
        e.preventDefault();
        const nextIndex = Math.min(s.results.items.length - 1, s.focusedIndex + 1);
        s.setFocusedIndex(nextIndex);
        const item = s.results.items[nextIndex];
        if (item) void s.select(item, nextIndex);
        return;
      }
      if (e.key === "ArrowUp" || e.key === "k") {
        if (!s.results.items.length) return;
        e.preventDefault();
        const nextIndex = Math.max(0, s.focusedIndex - 1);
        s.setFocusedIndex(nextIndex);
        const item = s.results.items[nextIndex];
        if (item) void s.select(item, nextIndex);
        return;
      }
      if (e.key === "Enter") {
        if (s.focusedIndex >= 0 && s.focusedIndex < s.results.items.length) {
          e.preventDefault();
          void s.select(s.results.items[s.focusedIndex], s.focusedIndex);
        }
        return;
      }
      if (e.key === "PageDown") {
        if (s.offset + 75 < s.results.total) {
          e.preventDefault();
          s.setOffset(s.offset + 75);
        }
        return;
      }
      if (e.key === "PageUp") {
        if (s.offset > 0) {
          e.preventDefault();
          s.setOffset(Math.max(0, s.offset - 75));
        }
        return;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
