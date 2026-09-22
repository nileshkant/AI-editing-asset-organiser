import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  AudioLines,
  Check,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  FolderPlus,
  Heart,
  Inbox,
  Library,
  Pause,
  Play,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Square,
  Tag,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  call,
  duration,
  type AppInfo,
  type PlaybackStatus,
  type Progress,
  type SearchResults,
  type Sound,
  type Source,
} from "./api";
import { Waveform } from "./Waveform";

type Page = "library" | "favorites" | "imports" | "settings";
const EMPTY: SearchResults = {
  items: [],
  total: 0,
  interpretation: {
    terms: [],
    excluded: [],
    min_duration: null,
    max_duration: null,
    corrected: [],
  },
};
export function App() {
  const [page, setPage] = useState<Page>("library");
  const [info, setInfo] = useState<AppInfo>();
  const [roots, setRoots] = useState<Source[]>([]);
  const [jobs, setJobs] = useState<Progress[]>([]);
  const [text, setText] = useState("");
  const [source, setSource] = useState("");
  const [max, setMax] = useState("");
  const [offset, setOffset] = useState(0);
  const [results, setResults] = useState(EMPTY);
  const [selected, setSelected] = useState<Sound | null>(null);
  const [playingSound, setPlayingSound] = useState<Sound | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const [focusedIndex, setFocusedIndex] = useState<number>(-1);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [filters, setFilters] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [relink, setRelink] = useState<Source | null>(null);
  const [path, setPath] = useState("");
  const request = useRef(0);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [playback, setPlayback] = useState<PlaybackStatus | null>(null);
  const [volume, setVolumeState] = useState(1.0);
  const [muted, setMuted] = useState(false);
  const refresh = () => setRevision((r) => r + 1);
  const guard = async <T,>(task: () => Promise<T>): Promise<T | undefined> => {
    try {
      setError("");
      return await task();
    } catch (e) {
      setError(String(e));
      return undefined;
    }
  };
  async function importPath(path: string) {
    await call("import_root", { path });
    refresh();
  }
  const addFolder = () =>
    guard(async () => {
      const path = await call<string | null>("choose_folder");
      if (path) await importPath(path);
    });

  const playSound = (sound: Sound) =>
    guard(async () => {
      setPlayingSound(sound);
      setAnnouncement(`Playing ${sound.title}`);
      await call("playback_play", { id: sound.id });
    });
  const togglePlay = () =>
    guard(async () => {
      if (
        !playback ||
        playback.state === "stopped" ||
        playback.state === "finished"
      ) {
        if (playingSound) await playSound(playingSound);
        else if (selected) await playSound(selected);
        else if (results.items.length > 0) await playSound(results.items[0]);
      } else if (playback.state === "playing") {
        setAnnouncement("Playback paused");
        await call("playback_pause");
      } else if (playback.state === "paused") {
        setAnnouncement("Playback resumed");
        await call("playback_resume");
      }
    });
  const stopPlayback = () =>
    guard(async () => {
      setAnnouncement("Playback stopped");
      await call("playback_stop");
    });
  const seekPlayback = (pos: number) =>
    guard(async () => {
      await call("playback_seek", { positionSeconds: pos });
    });
  const changeVolume = (v: number) =>
    guard(async () => {
      setVolumeState(v);
      setMuted(v === 0);
      await call("playback_set_volume", { volume: v });
    });
  const toggleMute = () =>
    guard(async () => {
      const next = !muted;
      setMuted(next);
      setAnnouncement(next ? "Muted" : "Unmuted");
      await call("playback_set_volume", { volume: next ? 0 : volume });
    });

  useEffect(() => {
    if (!isTauri()) return;
    void guard(async () => setInfo(await call("app_info")));
  }, []);
  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    let previous = "";
    const update = async () => {
      try {
        const list = await call<Progress[]>("jobs");
        if (!alive) return;
        setJobs(list);
        const signature = JSON.stringify(list);
        if (signature !== previous) {
          previous = signature;
          refresh();
        }
        setRoots(await call("sources"));
      } catch (e) {
        if (alive) setError(String(e));
      }
    };
    void update();
    const timer = setInterval(update, 1500);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, []);
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
    const inList = results.items.find((s) => s.id === playback.sound_id);
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
  }, [playback?.sound_id, results.items, selected]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName?.toLowerCase();
      const isInput =
        tag === "input" ||
        tag === "textarea" ||
        tag === "select" ||
        (e.target as HTMLElement)?.isContentEditable;
      if (e.key === "Escape") {
        if (relink) {
          setRelink(null);
          return;
        }
        if (selected) {
          setSelected(null);
          setAnnouncement("Details closed");
          return;
        }
        if (error) {
          setError("");
          return;
        }
        if (filters) {
          setFilters(false);
          return;
        }
        return;
      }
      if (isInput) return;
      if (e.code === "Space") {
        e.preventDefault();
        void togglePlay();
        return;
      }
      if (e.key === "/" && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        searchInputRef.current?.focus();
        return;
      }
      if (e.key === "m" || e.key === "M") {
        e.preventDefault();
        void toggleMute();
        return;
      }
      if (e.key === "ArrowDown" || e.key === "j") {
        if (!results.items.length) return;
        e.preventDefault();
        const nextIndex = Math.min(results.items.length - 1, focusedIndex + 1);
        setFocusedIndex(nextIndex);
        const item = results.items[nextIndex];
        if (item) void select(item, nextIndex);
        return;
      }
      if (e.key === "ArrowUp" || e.key === "k") {
        if (!results.items.length) return;
        e.preventDefault();
        const nextIndex = Math.max(0, focusedIndex - 1);
        setFocusedIndex(nextIndex);
        const item = results.items[nextIndex];
        if (item) void select(item, nextIndex);
        return;
      }
      if (e.key === "Enter") {
        if (focusedIndex >= 0 && focusedIndex < results.items.length) {
          e.preventDefault();
          void select(results.items[focusedIndex], focusedIndex);
        }
        return;
      }
      if (e.key === "PageDown") {
        if (offset + 75 < results.total) {
          e.preventDefault();
          setOffset(offset + 75);
        }
        return;
      }
      if (e.key === "PageUp") {
        if (offset > 0) {
          e.preventDefault();
          setOffset(Math.max(0, offset - 75));
        }
        return;
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    playback,
    selected,
    playingSound,
    results.items,
    focusedIndex,
    offset,
    relink,
    error,
    filters,
  ]);

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        if (disposed) return;
        const payload = event.payload;
        setDropping(payload.type === "over" || payload.type === "enter");
        if (payload.type === "drop") {
          const paths = payload.paths;
          void guard(async () => {
            for (const path of paths) await importPath(path);
          });
        }
      })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch((e) => setError(String(e)));
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  useEffect(() => {
    setOffset(0);
    setFocusedIndex(-1);
  }, [text, source, max, page]);
  useEffect(() => {
    if (!isTauri()) return;
    const id = ++request.current;
    const timer = setTimeout(() => {
      setBusy(true);
      call<SearchResults>("search_sounds", {
        query: {
          text,
          source_ids: source ? [source] : [],
          max_duration: max ? Number(max) : null,
          favorites_only: page === "favorites",
          offset,
          limit: 75,
        },
      })
        .then((r) => {
          if (request.current === id) {
            setResults(r);
            setAnnouncement(
              r.total === 0
                ? "No sounds found"
                : r.total === 1
                  ? "1 sound found"
                  : `${r.total.toLocaleString()} sounds found`,
            );
            setSelected((curr) => {
              if (curr)
                call<Sound>("get_sound", { id: curr.id }).catch(() =>
                  setSelected(null),
                );
              return curr;
            });
          }
        })
        .catch((e) => {
          if (request.current === id) setError(String(e));
        })
        .finally(() => {
          if (request.current === id) setBusy(false);
        });
    }, 180);
    return () => clearTimeout(timer);
  }, [text, source, max, page, offset, revision]);
  const select = (sound: Sound, index?: number) =>
    guard(async () => {
      if (typeof index === "number") setFocusedIndex(index);
      setAnnouncement(`Selected ${sound.title}`);
      const id = ++selectionId.current;
      const full = await call<Sound>("get_sound", { id: sound.id });
      if (selectionId.current === id) setSelected(full);
    });
  const selectionId = useRef(0);
  const favorite = (sound: Sound) =>
    guard(async () => {
      await call("annotate", {
        id: sound.id,
        tags: sound.user_tags,
        comment: sound.comment,
        favorite: !sound.favorite,
      });
      if (selected?.id === sound.id)
        setSelected({ ...selected, favorite: !sound.favorite });
      refresh();
    });
  const active = jobs.filter((j) =>
    ["queued", "discovering", "analyzing"].includes(j.status),
  );
  return (
    <div className="app-shell">
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        aria-label="Announcements"
        className="sr-only"
      >
        {announcement}
      </div>
      <aside className="sidebar">
        <div className="brand">
          <AudioLines size={25} aria-hidden="true" />
          <strong>SoundShelf</strong>
        </div>
        <nav aria-label="Main navigation">
          {(
            [
              { id: "library", label: "Library", icon: Library },
              { id: "favorites", label: "Favorites", icon: Heart },
              { id: "imports", label: "Imports", icon: Inbox },
              { id: "settings", label: "Settings", icon: Settings2 },
            ] as const
          ).map((n) => (
            <button
              key={n.id}
              className={`nav ${page === n.id ? "active" : ""}`}
              aria-current={page === n.id ? "page" : undefined}
              onClick={() => setPage(n.id)}
            >
              <n.icon size={18} aria-hidden="true" />
              <span>{n.label}</span>
              {n.id === "imports" && active.length > 0 && (
                <span className="count">{active.length}</span>
              )}
            </button>
          ))}
        </nav>
        <div className="source-heading">
          FOLDERS
          <button
            className="icon-button"
            title="Add folder"
            aria-label="Add folder"
            onClick={addFolder}
          >
            <FolderPlus size={15} aria-hidden="true" />
          </button>
        </div>
        <div className="source-links">
          {roots.map((root) => (
            <button
              key={root.id}
              className={`source-link ${source === root.id ? "chosen" : ""}`}
              title={root.root}
              onClick={() => {
                setSource(source === root.id ? "" : root.id);
                setPage("library");
              }}
            >
              <FolderOpen size={15} aria-hidden="true" />
              <span>{root.name}</span>
              {!root.available && (
                <span
                  className="offline-dot"
                  title="Folder offline"
                  aria-label="Folder offline"
                />
              )}
            </button>
          ))}
        </div>
        <div className="sidebar-bottom">
          <ShieldCheck size={15} aria-hidden="true" />
          <span>Local workspace</span>
          <span className="status-dot" aria-hidden="true" />
        </div>
      </aside>
      <main>
        <header className="page-header">
          <div>
            <p className="eyebrow">YOUR AUDIO WORKSPACE</p>
            <h1>
              {page === "library"
                ? "Library"
                : page[0].toUpperCase() + page.slice(1)}
            </h1>
          </div>
          <div className="header-actions">
            <span className="build-label">Development build</span>
            <button className="primary" onClick={addFolder}>
              <FolderPlus size={17} aria-hidden="true" />
              Add folder
            </button>
          </div>
        </header>
        {error && (
          <div className="error" role="alert">
            <span>{error}</span>
            <button
              className="icon-button"
              aria-label="Dismiss error"
              onClick={() => setError("")}
            >
              <X size={15} aria-hidden="true" />
            </button>
          </div>
        )}
        {(page === "library" || page === "favorites") && (
          <>
            <div className="library-toolbar">
              <div className="search-input">
                <Search size={18} aria-hidden="true" />
                <input
                  ref={searchInputRef}
                  aria-label="Search audio"
                  placeholder="Search sounds, tags, or a short whoosh under 3 seconds (Press '/' to focus)"
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                />
                {text && (
                  <button
                    className="icon-button"
                    aria-label="Clear search"
                    onClick={() => setText("")}
                  >
                    <X size={15} aria-hidden="true" />
                  </button>
                )}
              </div>
              <button
                className={`icon-button ${filters ? "chosen" : ""}`}
                aria-label="Filters"
                aria-pressed={filters}
                aria-expanded={filters}
                title="Filters"
                onClick={() => setFilters(!filters)}
              >
                <SlidersHorizontal size={18} aria-hidden="true" />
              </button>
            </div>
            {filters && (
              <div
                className="filter-strip"
                role="region"
                aria-label="Search filters"
              >
                <label>
                  Folder
                  <select
                    value={source}
                    onChange={(e) => setSource(e.target.value)}
                  >
                    <option value="">All folders</option>
                    {roots.map((r) => (
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
                    value={max}
                    onChange={(e) => setMax(e.target.value)}
                  />
                </label>
                <button
                  onClick={() => {
                    setSource("");
                    setMax("");
                    setAnnouncement("Filters reset");
                  }}
                >
                  Reset
                </button>
              </div>
            )}
            <div className="results-bar">
              <span>
                {busy
                  ? "Searching..."
                  : `${results.total.toLocaleString()} sounds`}
              </span>
              <span>
                {results.interpretation.corrected.join(" · ") ||
                  "Offline search"}
              </span>
            </div>
            <div className={`library-body ${selected ? "with-inspector" : ""}`}>
              <section
                className="sound-list"
                role="listbox"
                aria-label="Sounds"
              >
                {results.items.length ? (
                  results.items.map((sound, index) => {
                    const isThisPlaying =
                      playback?.sound_id === sound.id &&
                      playback?.state === "playing";
                    const isThisActive = playback?.sound_id === sound.id;
                    return (
                      <div
                        key={sound.id}
                        role="option"
                        aria-selected={selected?.id === sound.id}
                        aria-current={isThisPlaying ? "true" : undefined}
                        tabIndex={0}
                        onFocus={() => setFocusedIndex(index)}
                        className={`sound-row ${selected?.id === sound.id ? "selected" : ""} ${isThisPlaying ? "playing" : ""} ${focusedIndex === index ? "keyboard-focused" : ""}`}
                      >
                        <button
                          className="sound-select"
                          onClick={() => select(sound, index)}
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
                            <small>
                              {[
                                ...(sound.user_tags.length
                                  ? sound.user_tags
                                  : sound.profile?.tags || []),
                              ]
                                .slice(0, 3)
                                .map((t) => t.replaceAll("_", " "))
                                .join(" · ")}
                            </small>
                          </span>
                          <span className="sound-duration">
                            {duration(sound.profile?.duration || 0)}
                          </span>
                        </button>
                        <button
                          className="icon-button"
                          title={
                            isThisPlaying
                              ? `Pause ${sound.title}`
                              : `Play ${sound.title}`
                          }
                          aria-label={
                            isThisPlaying
                              ? `Pause ${sound.title}`
                              : `Play ${sound.title}`
                          }
                          onClick={(e) => {
                            e.stopPropagation();
                            if (isThisPlaying) {
                              void call("playback_pause");
                              setAnnouncement(`Paused ${sound.title}`);
                            } else if (
                              isThisActive &&
                              playback?.state === "paused"
                            ) {
                              void call("playback_resume");
                              setAnnouncement(`Resumed ${sound.title}`);
                            } else {
                              void playSound(sound);
                            }
                          }}
                        >
                          {isThisPlaying ? (
                            <Pause size={15} aria-hidden="true" />
                          ) : (
                            <Play size={15} aria-hidden="true" />
                          )}
                        </button>
                        <button
                          className="icon-button favorite"
                          title={
                            sound.favorite ? "Remove favorite" : "Add favorite"
                          }
                          aria-label={`${sound.favorite ? "Unfavorite" : "Favorite"} ${sound.title}`}
                          aria-pressed={sound.favorite}
                          onClick={() => favorite(sound)}
                        >
                          <Heart
                            size={16}
                            fill={sound.favorite ? "currentColor" : "none"}
                            aria-hidden="true"
                          />
                        </button>
                      </div>
                    );
                  })
                ) : (
                  <div className="empty">
                    <FolderOpen size={48} strokeWidth={1} aria-hidden="true" />
                    <h2>
                      {text || source || page === "favorites"
                        ? "No matching sounds"
                        : "No sounds yet"}
                    </h2>
                    <button onClick={addFolder}>
                      <FolderPlus size={16} aria-hidden="true" />
                      Add folder
                    </button>
                  </div>
                )}
              </section>
              {selected && (
                <SoundInspector
                  key={selected.id}
                  sound={selected}
                  onClose={() => setSelected(null)}
                  onSave={async (tags, comment) => {
                    await call("annotate", {
                      id: selected.id,
                      tags,
                      comment,
                      favorite: selected.favorite,
                    });
                    const full = await call<Sound>("get_sound", {
                      id: selected.id,
                    });
                    setSelected(full);
                    setAnnouncement("Annotations saved");
                    refresh();
                  }}
                  onError={(e) => setError(e)}
                />
              )}
            </div>
            <div
              className="pagination"
              role="navigation"
              aria-label="Pagination"
            >
              <span>
                {results.total
                  ? `${offset + 1}-${Math.min(offset + 75, results.total)} of ${results.total}`
                  : "0 results"}
              </span>
              <button
                className="icon-button"
                title="Previous page"
                aria-label="Previous page"
                disabled={offset === 0}
                onClick={() => setOffset(Math.max(0, offset - 75))}
              >
                <ChevronLeft size={16} aria-hidden="true" />
              </button>
              <button
                className="icon-button"
                title="Next page"
                aria-label="Next page"
                disabled={offset + 75 >= results.total}
                onClick={() => setOffset(offset + 75)}
              >
                <ChevronRight size={16} aria-hidden="true" />
              </button>
            </div>
          </>
        )}
        {page === "imports" && (
          <section className="settings-body">
            {!jobs.length ? (
              <div className="empty">
                <Inbox size={40} aria-hidden="true" />
                <h2>No imports</h2>
              </div>
            ) : (
              jobs.map((job) => (
                <div className="job-row" key={job.source_id}>
                  <div className="job-heading">
                    <strong>
                      {roots.find((r) => r.id === job.source_id)?.name ||
                        "Folder"}
                    </strong>
                    <span>{job.status}</span>
                  </div>
                  <progress
                    value={job.completed}
                    max={Math.max(1, job.total)}
                  />
                  <p>
                    {job.completed} / {job.total} files · {job.reused} reused ·{" "}
                    {job.failed} failed
                  </p>
                  <small className="muted path-text">{job.current}</small>
                  {job.errors.length > 0 && (
                    <details>
                      <summary>Errors ({job.errors.length})</summary>
                      {job.errors.map((e, i) => (
                        <p key={i} className="path-text">
                          {e}
                        </p>
                      ))}
                    </details>
                  )}
                  {job.status === "analyzing" && (
                    <button onClick={() => guard(() => call("cancel_import"))}>
                      Cancel import
                    </button>
                  )}
                </div>
              ))
            )}
          </section>
        )}
        {page === "settings" && (
          <section className="settings-body">
            <h2>Folders</h2>
            {roots.length ? (
              roots.map((root) => (
                <div className="setting-row" key={root.id}>
                  <div className="folder-detail">
                    <strong>{root.name}</strong>
                    <code>{root.root}</code>
                    <small className="muted">
                      {root.available ? "Available" : "Offline"}
                    </small>
                  </div>
                  <div className="header-actions">
                    <button
                      className="icon-button"
                      title="Rescan folder"
                      aria-label={`Rescan ${root.name}`}
                      onClick={() =>
                        guard(() => call("scan_source", { id: root.id }))
                      }
                    >
                      <RefreshCw size={16} aria-hidden="true" />
                    </button>
                    <button
                      onClick={() => {
                        setRelink(root);
                        setPath(root.root);
                      }}
                    >
                      Relink
                    </button>
                  </div>
                </div>
              ))
            ) : (
              <p className="muted">No folders added.</p>
            )}
            <h2>Intelligence</h2>
            <div className="setting-row">
              <span>AI features</span>
              <span className="muted">
                Disabled · AI API or local model needed
              </span>
            </div>
            <h2>Application</h2>
            <div className="setting-row">
              <span>Version</span>
              <span>{info?.version || "0.1.0"} · Development</span>
            </div>
            <div className="setting-row">
              <span>Media engine</span>
              <span>{info?.media_tools ? "Available" : "Unavailable"}</span>
            </div>
            <div className="setting-row">
              <span>Data directory</span>
              <code>{info?.data_directory || "Desktop application only"}</code>
            </div>
          </section>
        )}
        <footer className="transport" aria-label="Audio transport">
          <div className="transport-controls">
            <button
              className="icon-button"
              title={playback?.state === "playing" ? "Pause" : "Play"}
              aria-label={playback?.state === "playing" ? "Pause" : "Play"}
              onClick={togglePlay}
            >
              {playback?.state === "playing" ? (
                <Pause size={18} aria-hidden="true" />
              ) : (
                <Play size={18} aria-hidden="true" />
              )}
            </button>
            <button
              className="icon-button"
              title="Stop"
              aria-label="Stop"
              disabled={!playback || playback.state === "stopped"}
              onClick={stopPlayback}
            >
              <Square size={16} aria-hidden="true" />
            </button>
          </div>
          <div className="transport-info">
            <strong
              className="transport-title"
              title={
                playingSound?.title ||
                (playback?.sound_id
                  ? "Playing audio"
                  : selected?.title || "No audio selected")
              }
            >
              {playingSound?.title ||
                (playback?.sound_id
                  ? "Playing audio"
                  : selected?.title || "No audio selected")}
            </strong>
            <small className="transport-meta">
              {playback?.error ? (
                <span style={{ color: "#ff9999" }}>{playback.error}</span>
              ) : playback?.state === "playing" ? (
                "Playing"
              ) : playback?.state === "paused" ? (
                "Paused"
              ) : playback?.state === "finished" ? (
                "Finished"
              ) : (
                "Stopped"
              )}
            </small>
          </div>
          <div className="transport-scrub">
            <span className="transport-time">
              {duration(playback?.position_seconds || 0)}
            </span>
            <input
              type="range"
              className="transport-slider"
              aria-label="Seek position"
              aria-valuemin={0}
              aria-valuemax={
                playback?.duration_seconds ||
                playingSound?.profile?.duration ||
                selected?.profile?.duration ||
                1
              }
              aria-valuenow={playback?.position_seconds || 0}
              aria-valuetext={`${duration(playback?.position_seconds || 0)} of ${duration(playback?.duration_seconds || playingSound?.profile?.duration || selected?.profile?.duration || 0)}`}
              min="0"
              max={
                playback?.duration_seconds ||
                playingSound?.profile?.duration ||
                selected?.profile?.duration ||
                1
              }
              step="0.05"
              value={playback?.position_seconds || 0}
              onChange={(e) => seekPlayback(Number(e.target.value))}
            />
            <span className="transport-time">
              {duration(
                playback?.duration_seconds ||
                  playingSound?.profile?.duration ||
                  selected?.profile?.duration ||
                  0,
              )}
            </span>
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
              title={muted || volume === 0 ? "Unmute" : "Mute"}
              aria-label={muted || volume === 0 ? "Unmute" : "Mute"}
              onClick={toggleMute}
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
              aria-valuenow={Math.round((muted ? 0 : volume) * 100)}
              aria-valuetext={`${Math.round((muted ? 0 : volume) * 100)} percent`}
              min="0"
              max="1"
              step="0.02"
              value={muted ? 0 : volume}
              onChange={(e) => changeVolume(Number(e.target.value))}
            />
          </div>
        </footer>
      </main>
      {dropping && (
        <div className="drop-overlay">
          <FolderPlus size={48} aria-hidden="true" />
          <strong>Add audio folders</strong>
        </div>
      )}
      {relink && (
        <div className="modal-backdrop">
          <section
            className="modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="relink-title"
          >
            <h2 id="relink-title">Relink {relink.name}</h2>
            <label>
              Folder path
              <input
                autoFocus
                value={path}
                onChange={(e) => setPath(e.target.value)}
              />
            </label>
            <div className="modal-actions">
              <button onClick={() => setRelink(null)}>Cancel</button>
              <button
                onClick={() =>
                  guard(async () => {
                    const p = await call<string | null>("choose_folder");
                    if (p) setPath(p);
                  })
                }
              >
                <FolderOpen size={16} aria-hidden="true" />
                Browse
              </button>
              <button
                className="primary"
                onClick={() =>
                  guard(async () => {
                    await call("relink_source", { id: relink.id, path });
                    setRelink(null);
                    setRoots(await call("sources"));
                    refresh();
                  })
                }
              >
                Verify & relink
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}

function SoundInspector({
  sound,
  onClose,
  onSave,
  onError,
}: {
  sound: Sound;
  onClose: () => void;
  onSave: (tags: string[], comment: string) => Promise<void>;
  onError: (e: string) => void;
}) {
  const [tags, setTags] = useState(sound.user_tags);
  const [tag, setTag] = useState("");
  const [comment, setComment] = useState(sound.comment);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const p = sound.profile;
  const addTag = () => {
    const next = tag.replaceAll("_", " ").trim();
    if (next && !tags.some((t) => t.toLowerCase() === next.toLowerCase()))
      setTags([...tags, next]);
    setTag("");
    setSaved(false);
  };
  return (
    <aside className="inspector" aria-labelledby="inspector-title">
      <div className="inspector-heading">
        <span>DETAILS</span>
        <button
          className="icon-button"
          title="Close details"
          aria-label="Close details"
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </div>
      <h2 id="inspector-title">{sound.title}</h2>
      <small className="muted path-text">{sound.relative_path}</small>
      {p && (
        <>
          <Waveform peaks={p.waveform} />
          <div className="audio-facts">
            <span>{duration(p.duration)}</span>
            <span>{(p.sample_rate / 1000).toFixed(1)} kHz</span>
            <span>
              {p.channel_layout ||
                (p.channels === 1
                  ? "mono"
                  : p.channels === 2
                    ? "stereo"
                    : `${p.channels} ch`)}
            </span>
          </div>
          <h3>Measured profile</h3>
          <p className="description">{p.description}</p>
          <div className="tags">
            {p.tags.map((t) => (
              <span key={t} className="tag measured">
                {t.replaceAll("_", " ")}
              </span>
            ))}
          </div>
        </>
      )}
      <h3>Your tags</h3>
      <div className="tags">
        {tags.map((t) => (
          <span className="tag" key={t}>
            {t}
            <button
              aria-label={`Remove tag ${t}`}
              onClick={() => {
                setTags(tags.filter((x) => x !== t));
                setSaved(false);
              }}
            >
              <X size={12} />
            </button>
          </span>
        ))}
      </div>
      <form
        className="tag-form"
        onSubmit={(e) => {
          e.preventDefault();
          addTag();
        }}
      >
        <Tag size={15} />
        <input
          aria-label="New tag"
          placeholder="Add a tag"
          maxLength={64}
          value={tag}
          onChange={(e) => setTag(e.target.value)}
        />
        <button
          className="icon-button"
          type="submit"
          title="Add tag"
          aria-label="Add tag"
        >
          <Check size={15} />
        </button>
      </form>
      <label className="comment-label">
        Comment
        <textarea
          aria-label="Comment"
          value={comment}
          maxLength={10000}
          onChange={(e) => {
            setComment(e.target.value);
            setSaved(false);
          }}
          rows={4}
        />
      </label>
      <button
        className="save-metadata"
        disabled={saving}
        onClick={async () => {
          setSaving(true);
          try {
            await onSave(tags, comment);
            setSaved(true);
          } catch (e) {
            onError(String(e));
          } finally {
            setSaving(false);
          }
        }}
      >
        <Check size={15} />
        {saving ? "Saving..." : saved ? "Saved" : "Save changes"}
      </button>
    </aside>
  );
}
