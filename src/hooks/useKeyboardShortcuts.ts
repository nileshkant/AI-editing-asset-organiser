import { useEffect, useRef } from 'react';
import { Page, PlaybackStatus, SearchResults, Sound, Source } from '../api';

export interface KeyboardShortcutsConfig {
  page: Page;
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
  
  stateRef.current = config;
  
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const s = stateRef.current;
      // Settings owns its keyboard controls. Library shortcuts must not steal
      // Space/Enter or scrolling keys and unexpectedly play/select audio there.
      if (e.defaultPrevented || s.page === 'settings') return;
      const target = e.target instanceof HTMLElement ? e.target : null;
      if (target?.closest('[role="dialog"], dialog') || s.relink) return;
      if (e.ctrlKey || e.metaKey || e.altKey || e.isComposing) return;
      const tag = target?.tagName?.toLowerCase();
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
        // The image view owns Escape; hidden audio details stay open for return.
        if (s.selected && s.page !== 'images') {
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
      if (e.key === "/" && !e.ctrlKey && !e.metaKey && s.page !== 'images') {
        e.preventDefault();
        s.searchInputRef.current?.focus();
        return;
      }

      // Native controls own Space/Enter and their navigation keys. Only a
      // sound's title button participates in result navigation.
      const interactive = target?.closest('button, a, [role="button"], [role="tab"]');
      const rowNavigation = target?.closest('.sound-select') &&
        ['ArrowDown', 'ArrowUp', 'Home', 'End', 'j', 'k'].includes(e.key);
      if (interactive && !rowNavigation) return;
      if (target?.closest('.inspector, .waveform-workstation, .image-viewer')) return;
      if (e.code === "Space") {
        e.preventDefault();
        void s.togglePlay();
        return;
      }
      if (e.key === "m" || e.key === "M") {
        e.preventDefault();
        void s.toggleMute();
        return;
      }
      if (s.page !== 'library' && s.page !== 'favorites') return;
      const focusResult = (index: number) => {
        s.setFocusedIndex(index);
        const button = document.querySelectorAll<HTMLButtonElement>('.sound-select')[index];
        button?.focus();
        button?.scrollIntoView?.({ block: 'nearest' });
      };
      if (e.key === 'Home' || e.key === 'End') {
        if (!s.results.items.length) return;
        e.preventDefault();
        const index = e.key === 'Home' ? 0 : s.results.items.length - 1;
        focusResult(index);
        return;
      }
      if (e.key === "ArrowDown" || e.key === "j") {
        if (!s.results.items.length) return;
        e.preventDefault();
        const nextIndex = Math.min(s.results.items.length - 1, s.focusedIndex + 1);
        focusResult(nextIndex);
        return;
      }
      if (e.key === "ArrowUp" || e.key === "k") {
        if (!s.results.items.length) return;
        e.preventDefault();
        const nextIndex = Math.max(0, s.focusedIndex - 1);
        focusResult(nextIndex);
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
