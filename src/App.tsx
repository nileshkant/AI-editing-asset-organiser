import { useCallback, useMemo, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, FolderPlus, Search, SlidersHorizontal, X } from 'lucide-react';
import { call } from './api';
import { usePlayback } from './hooks/usePlayback';
import { useAudioSearch } from './hooks/useAudioSearch';
import { useKeyboardShortcuts } from './hooks/useKeyboardShortcuts';
import { useImageImports } from './hooks/useImageImports';
import { ImageImports } from './components/views/ImageImports';
import { useDragDrop } from './hooks/useDragDrop';
import { Sidebar } from './components/layout/Sidebar';
import { PageHeader } from './components/layout/PageHeader';
import { SoundList } from './components/sound/SoundList';
import { SoundInspector } from './components/sound/SoundInspector';
import { TransportBar } from './components/transport/TransportBar';
import { ImportsView } from './components/views/ImportsView';
import { SettingsView } from './components/views/SettingsView';
import { RelinkModal } from './components/modals/RelinkModal';
import { ImageGallery } from './components/images/ImageGallery';
import { useImageGallery } from './hooks/useImageGallery';
import type { Page, Sound, Source } from './types';

export function App() {
  const [page, setPage] = useState<Page>('library');
  // The image view keeps the last audio page's search, offset and selection intact.
  const [audioPage, setAudioPage] = useState<'library' | 'favorites'>('library');
  const [selected, setSelected] = useState<Sound | null>(null);
  const [announcement, setAnnouncement] = useState('');
  const [focusedIndex, setFocusedIndex] = useState(-1);
  const [error, setError] = useState('');
  const [relink, setRelink] = useState<Source | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const selectionId = useRef(0);

  // Custom hooks
  const search = useAudioSearch(page === 'images' ? audioPage : page);
  const playback = usePlayback(selected, search.results.items);
  const openImports = useCallback(() => setPage('imports'), []);
  const imageImports = useImageImports(setError, openImports);
  const imagesVisible = imageImports.enabled || imageImports.metadataPresent;
  const imageRevision = useMemo(() => imageImports.jobs.map(j => `${j.id}:${j.sequence}`).join(','), [imageImports.jobs]);
  const gallery = useImageGallery(imageRevision, imageImports.resets);
  const { dropping } = useDragDrop(search.importPath, setError, { ready: imageImports.available, enabled: imageImports.enabled });

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

  const closeDetails = useCallback(() => {
    ++selectionId.current;
    setSelected(null);
    const previousFocus = document.activeElement;
    requestAnimationFrame(() => {
      // A newer keyboard action (for example / to search) owns its focus.
      if (document.activeElement === previousFocus || document.activeElement === document.body)
        document.querySelectorAll<HTMLButtonElement>('.sound-select')[focusedIndex]?.focus();
    });
  }, [focusedIndex]);

  const select = useCallback(
    (sound: Sound, index?: number) =>
      guard(async () => {
        if (typeof index === 'number') setFocusedIndex(index);
        setSelected(sound);
        setAnnouncement(`Selected ${sound.title}`);
        requestAnimationFrame(() => document.querySelector<HTMLElement>('.inspector')?.focus());
        const id = ++selectionId.current;
        const full = await call<Sound>('get_sound', { id: sound.id });
        if (selectionId.current === id) setSelected(full);
      }),
    [guard],
  );

  const favorite = useCallback(
    (sound: Sound) =>
      guard(async () => {
        await call('set_favorite', {
          id: sound.id,
          favorite: !sound.favorite,
        });
        setSelected(current => current?.id === sound.id ? { ...current, favorite: !sound.favorite } : current);
        search.refresh();
      }),
    [guard, search.refresh],
  );

  const handleRescan = useCallback(
    (id: string) => guard(() => call('scan_source', { id })),
    [guard],
  );

  const handleCancelImport = useCallback(
    () => guard(() => call('cancel_import')),
    [guard],
  );

  // Keyboard shortcuts
  useKeyboardShortcuts({
    page,
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
    setSelected: (sound) => { if (!sound) closeDetails(); else setSelected(sound); },
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
      <a className="skip-link" href="#workspace">Skip to workspace</a>
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
      <div role="status" aria-live="polite" aria-atomic="true" aria-label="Playback announcements" className="sr-only">{playback.announcement}</div>
      <div role="status" aria-live="polite" aria-atomic="true" aria-label="Search announcements" className="sr-only">{search.announcement}</div>

      <Sidebar
        page={page}
        setPage={(next) => {
          // Visiting images and returning keeps the audio selection and focus.
          if (next !== page && next !== 'images' && page !== 'images') {
            ++selectionId.current;
            setSelected(null);
            setFocusedIndex(-1);
          }
          if (next === 'library' || next === 'favorites') setAudioPage(next);
          setPage(next);
        }}
        images={imagesVisible}
        roots={search.roots}
        source={search.source}
        setSource={search.setSource}
        activeJobCount={activeJobCount}
        onAddFolder={search.addFolder}
      />

      <main id="workspace" tabIndex={-1}>
        <PageHeader page={page} onAddFolder={(page === 'imports' || page === 'images') && imageImports.enabled ? () => void imageImports.choose(true) : search.addFolder} onAddFiles={(page === 'imports' || page === 'images') && imageImports.enabled ? () => void imageImports.choose(false) : search.addFiles} />

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
                  placeholder="Search sounds, tags, or descriptions"
                  aria-describedby="library-keyboard-help"
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
                    <option value="">All sources</option>
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

            <p className="keyboard-help" id="library-keyboard-help">/ Search · ↑ ↓ Browse · Enter Details</p>
            <div className="results-bar">
              <span>
                {search.busy
                  ? 'Searching...'
                  : `${search.results.total.toLocaleString()} ${search.results.total === 1 ? 'sound' : 'sounds'}`}
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
                role="list"
                aria-label="Sounds"
                aria-busy={search.busy}
              >
                {search.busy && !search.results.items.length ? (
                  <div className="catalog-loading" role="status" aria-label="Loading sounds">
                    <div className="loading-wave" aria-hidden="true">
                      {Array.from({ length: 5 }, (_, i) => <span key={i} />)}
                    </div>
                    <span>Loading {page === 'favorites' ? 'favorites' : 'library'}…</span>
                  </div>
                ) : <SoundList
                  items={search.results.items}
                  playback={playback.playback}
                  selectedId={selected?.id || null}
                  focusedIndex={focusedIndex}
                  isPlayPending={playback.isPlayPending}
                  onSelect={select}
                  onPlay={playback.playSound}
                  onPause={playback.togglePlay}
                  onResume={playback.togglePlay}
                  onFavorite={favorite}
                  onFocusIndex={setFocusedIndex}
                  onAddFolder={search.addFolder}
                  hasSearchOrFilter={!!(search.text || search.source)}
                  isFavoritesPage={page === 'favorites'}
                />}
              </section>

              {selected && (
                <SoundInspector
                  onExported={search.refresh}
                  key={selected.id}
                  sound={selected}
                  playback={playback.playback}
                  onPlay={playback.playSound}
                  onSeek={playback.seekPlayback}
                  onClose={closeDetails}
                  onSave={async (tags, comment) => {
                    await call('annotate', {
                      id: selected.id,
                      tags,
                      comment,
                    });
                    const full = await call<Sound>('get_sound', {
                      id: selected.id,
                    });
                    setSelected(current => current?.id === full.id ? full : current);
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
                disabled={search.busy || search.offset === 0}
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
                disabled={search.busy || search.offset + 75 >= search.results.total}
                onClick={() => search.setOffset(search.offset + 75)}
              >
                <ChevronRight size={16} aria-hidden="true" />
              </button>
            </div>
          </>
        )}

        {page === 'images' && <ImageGallery gallery={gallery} sources={search.roots} jobs={imageImports.jobs} onOpenImports={openImports} />}

        {page === 'imports' && (
          <div className="imports-workspace">
          <ImageImports controller={imageImports} sources={search.roots} />
          {(search.jobs.length > 0 || (!imageImports.enabled && !imageImports.metadataPresent && imageImports.jobs.length === 0)) && <ImportsView
            jobs={search.jobs}
            roots={search.roots}
            onCancelImport={() => void handleCancelImport()}
          />}
          </div>
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
          pending={playback.isPlayPending}
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
          <strong>Add audio files or folders</strong>
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
