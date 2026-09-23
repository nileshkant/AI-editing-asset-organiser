import { useCallback, useMemo, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, FolderPlus, Search, SlidersHorizontal, X } from 'lucide-react';
import { call } from './api';
import { usePlayback } from './hooks/usePlayback';
import { useAudioSearch } from './hooks/useAudioSearch';
import { useKeyboardShortcuts } from './hooks/useKeyboardShortcuts';
import { useDragDrop } from './hooks/useDragDrop';
import { Sidebar } from './components/layout/Sidebar';
import { PageHeader } from './components/layout/PageHeader';
import { SoundList } from './components/sound/SoundList';
import { SoundInspector } from './components/sound/SoundInspector';
import { TransportBar } from './components/transport/TransportBar';
import { ImportsView } from './components/views/ImportsView';
import { SettingsView } from './components/views/SettingsView';
import { RelinkModal } from './components/modals/RelinkModal';
import type { Page, Sound, Source } from './types';

export function App() {
  const [page, setPage] = useState<Page>('library');
  const [selected, setSelected] = useState<Sound | null>(null);
  const [announcement, setAnnouncement] = useState('');
  const [focusedIndex, setFocusedIndex] = useState(-1);
  const [error, setError] = useState('');
  const [relink, setRelink] = useState<Source | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const selectionId = useRef(0);

  // Custom hooks
  const search = useAudioSearch(page, selected?.id, setSelected);
  const playback = usePlayback(selected, search.results.items);
  const { dropping } = useDragDrop(search.importPath, setError);

  const guard = useCallback(
    async <T,>(task: () => Promise<T>): Promise<T | undefined> => {
      try {
        setError('');
        return await task();
      } catch (e) {
        setError(String(e));
        return undefined;
      }
    },
    [],
  );

  // Fix 1: Optimistic selection — show immediately, merge full data after
  const select = useCallback(
    (sound: Sound, index?: number) =>
      guard(async () => {
        if (typeof index === 'number') setFocusedIndex(index);
        setSelected(sound); // ← Optimistic: instant UI feedback
        setAnnouncement(`Selected ${sound.title}`);
        const id = ++selectionId.current;
        const full = await call<Sound>('get_sound', { id: sound.id });
        if (selectionId.current === id) setSelected(full); // Merge full data
      }),
    [guard],
  );

  const favorite = useCallback(
    (sound: Sound) =>
      guard(async () => {
        await call('annotate', {
          id: sound.id,
          tags: sound.user_tags,
          comment: sound.comment,
          favorite: !sound.favorite,
        });
        if (selected?.id === sound.id)
          setSelected({ ...selected, favorite: !sound.favorite });
        search.refresh();
      }),
    [guard, selected, search],
  );

  const handleRescan = useCallback(
    (id: string) => guard(() => call('scan_source', { id })),
    [guard],
  );

  const handleCancelImport = useCallback(
    () => guard(() => call('cancel_import')),
    [guard],
  );

  const handlePause = useCallback(
    () => {
      void call('playback_pause');
      setAnnouncement('Paused');
    },
    [],
  );

  const handleResume = useCallback(
    () => {
      void call('playback_resume');
      setAnnouncement('Resumed');
    },
    [],
  );

  // Keyboard shortcuts
  useKeyboardShortcuts({
    playback: playback.playback,
    selected,
    playingSound: playback.playingSound,
    results: search.results,
    focusedIndex,
    offset: search.offset,
    relink,
    error,
    filters: search.filters,
    togglePlay: playback.togglePlay,
    toggleMute: playback.toggleMute,
    select,
    setSelected,
    setFocusedIndex,
    setOffset: search.setOffset,
    setRelink,
    setError,
    setFilters: search.setFilters,
    setAnnouncement,
    searchInputRef,
  });

  const activeJobCount = useMemo(
    () =>
      (search.jobs || []).filter((j) =>
        ['queued', 'discovering', 'analyzing'].includes(j.status),
      ).length,
    [search.jobs],
  );

  return (
    <div className="app-shell">
      {/* Live region for screen readers */}
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        aria-label="Announcements"
        className="sr-only"
      >
        {announcement}
      </div>

      <Sidebar
        page={page}
        setPage={setPage}
        roots={search.roots}
        source={search.source}
        setSource={search.setSource}
        activeJobCount={activeJobCount}
        onAddFolder={search.addFolder}
      />

      <main>
        <PageHeader page={page} onAddFolder={search.addFolder} />

        {(error || search.error || playback.error) && (
          <div className="error" role="alert">
            <span>{error || search.error || playback.error}</span>
            <button
              className="icon-button"
              aria-label="Dismiss error"
              onClick={() => {
                setError('');
                search.setError('');
                playback.setError('');
              }}
            >
              <X size={15} aria-hidden="true" />
            </button>
          </div>
        )}

        {(page === 'library' || page === 'favorites') && (
          <>
            {/* Search & Filters */}
            <div className="library-toolbar">
              <div className="search-input">
                <Search size={18} aria-hidden="true" />
                <input
                  ref={searchInputRef}
                  aria-label="Search audio"
                  placeholder="Search sounds, tags, or a short whoosh under 3 seconds (Press '/' to focus)"
                  value={search.text}
                  onChange={(e) => search.setText(e.target.value)}
                />
                {search.text && (
                  <button
                    className="icon-button"
                    aria-label="Clear search"
                    onClick={() => search.setText('')}
                  >
                    <X size={15} aria-hidden="true" />
                  </button>
                )}
              </div>
              <button
                className={`icon-button ${search.filters ? 'chosen' : ''}`}
                aria-label="Filters"
                aria-pressed={search.filters}
                aria-expanded={search.filters}
                title="Filters"
                onClick={() => search.setFilters(!search.filters)}
              >
                <SlidersHorizontal size={18} aria-hidden="true" />
              </button>
            </div>

            {search.filters && (
              <div
                className="filter-strip"
                role="region"
                aria-label="Search filters"
              >
                <label>
                  Folder
                  <select
                    value={search.source}
                    onChange={(e) => search.setSource(e.target.value)}
                  >
                    <option value="">All folders</option>
                    {search.roots.map((r) => (
                      <option key={r.id} value={r.id}>
                        {r.name}
                      </option>
                    ))}
                  </select>
                </label>
                <label>
                  Duration under
                  <input
                    type="number"
                    min="0.01"
                    step="0.1"
                    aria-label="Duration under seconds"
                    placeholder="seconds"
                    value={search.max}
                    onChange={(e) => search.setMax(e.target.value)}
                  />
                </label>
                <button
                  onClick={() => {
                    search.setSource('');
                    search.setMax('');
                    setAnnouncement('Filters reset');
                  }}
                >
                  Reset
                </button>
              </div>
            )}

            <div className="results-bar">
              <span>
                {search.busy
                  ? 'Searching...'
                  : `${search.results.total.toLocaleString()} sounds`}
              </span>
              <span>
                {search.results.interpretation.corrected.join(' · ') ||
                  'Offline search'}
              </span>
            </div>

            {/* Sound List + Inspector */}
            <div
              className={`library-body ${selected ? 'with-inspector' : ''}`}
            >
              <section
                className="sound-list"
                role="listbox"
                aria-label="Sounds"
              >
                <SoundList
                  items={search.results.items}
                  playback={playback.playback}
                  selectedId={selected?.id || null}
                  focusedIndex={focusedIndex}
                  isPlayPending={playback.isPlayPending}
                  onSelect={select}
                  onPlay={playback.playSound}
                  onPause={handlePause}
                  onResume={handleResume}
                  onFavorite={favorite}
                  onFocusIndex={setFocusedIndex}
                  onAddFolder={search.addFolder}
                  hasSearchOrFilter={!!(search.text || search.source)}
                  isFavoritesPage={page === 'favorites'}
                />
              </section>

              {selected && (
                <SoundInspector
                  key={selected.id}
                  sound={selected}
                  playback={playback.playback}
                  onPlay={playback.playSound}
                  onSeek={playback.seekPlayback}
                  onClose={() => setSelected(null)}
                  onSave={async (tags, comment) => {
                    await call('annotate', {
                      id: selected.id,
                      tags,
                      comment,
                      favorite: selected.favorite,
                    });
                    const full = await call<Sound>('get_sound', {
                      id: selected.id,
                    });
                    setSelected(full);
                    setAnnouncement('Annotations saved');
                    search.refresh();
                  }}
                  onError={setError}
                />
              )}
            </div>

            {/* Pagination */}
            <div
              className="pagination"
              role="navigation"
              aria-label="Pagination"
            >
              <span>
                {search.results.total
                  ? `${search.offset + 1}-${Math.min(search.offset + 75, search.results.total)} of ${search.results.total}`
                  : '0 results'}
              </span>
              <button
                className="icon-button"
                title="Previous page"
                aria-label="Previous page"
                disabled={search.offset === 0}
                onClick={() =>
                  search.setOffset(Math.max(0, search.offset - 75))
                }
              >
                <ChevronLeft size={16} aria-hidden="true" />
              </button>
              <button
                className="icon-button"
                title="Next page"
                aria-label="Next page"
                disabled={search.offset + 75 >= search.results.total}
                onClick={() => search.setOffset(search.offset + 75)}
              >
                <ChevronRight size={16} aria-hidden="true" />
              </button>
            </div>
          </>
        )}

        {page === 'imports' && (
          <ImportsView
            jobs={search.jobs}
            roots={search.roots}
            onCancelImport={() => void handleCancelImport()}
          />
        )}

        {page === 'settings' && (
          <SettingsView
            roots={search.roots}
            info={search.info}
            onRescan={(id) => void handleRescan(id)}
            onRelink={(root) => {
              setRelink(root);
            }}
            onError={setError}
          />
        )}

        <TransportBar
          playback={playback.playback}
          playingSound={playback.playingSound}
          selected={selected}
          volume={playback.volume}
          muted={playback.muted}
          onTogglePlay={playback.togglePlay}
          onStop={playback.stopPlayback}
          onSeek={playback.seekPlayback}
          onChangeVolume={playback.changeVolume}
          onToggleMute={playback.toggleMute}
        />
      </main>

      {/* Overlays */}
      {dropping && (
        <div className="drop-overlay">
          <FolderPlus size={48} aria-hidden="true" />
          <strong>Add audio folders</strong>
        </div>
      )}

      {relink && (
        <RelinkModal
          source={relink}
          onClose={() => setRelink(null)}
          onRelinked={async () => {
            setRelink(null);
            search.refresh();
          }}
          onError={setError}
        />
      )}
    </div>
  );
}
