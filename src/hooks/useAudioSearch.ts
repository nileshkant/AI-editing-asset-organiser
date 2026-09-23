import { useState, useEffect, useRef, useCallback } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { call, SearchResults, Sound, Source, Progress, AppInfo } from '../api';

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

type Page = "library" | "favorites" | "imports" | "settings";

export function useAudioSearch(page: Page, selectedId: string | undefined, setSelected: (s: Sound | null) => void) {
  const [info, setInfo] = useState<AppInfo>();
  const [roots, setRoots] = useState<Source[]>([]);
  const [jobs, setJobs] = useState<Progress[]>([]);
  
  const [text, setText] = useState("");
  const [source, setSource] = useState("");
  const [max, setMax] = useState("");
  const [offset, setOffset] = useState(0);
  const [results, setResults] = useState<SearchResults>(EMPTY);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [filters, setFilters] = useState(false);
  
  const [error, setError] = useState("");
  const [announcement, setAnnouncement] = useState("");

  const request = useRef(0);

  const refresh = useCallback(() => setRevision((r) => r + 1), []);

  const guard = async <T,>(task: () => Promise<T>): Promise<T | undefined> => {
    try {
      setError("");
      return await task();
    } catch (e) {
      setError(String(e));
      return undefined;
    }
  };

  const importPath = useCallback(async (path: string) => {
    await call("import_root", { path });
    refresh();
  }, [refresh]);

  const addFolder = useCallback(() =>
    guard(async () => {
      const path = await call<string | null>("choose_folder");
      if (path) await importPath(path);
    }), [importPath]);

  useEffect(() => {
    if (!isTauri()) return;
    void guard(async () => setInfo(await call("app_info")));
  }, []);

  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    // Track the previous "status-level" signature: only job status transitions trigger a
    // library refresh. Heartbeat-level changes (completed counter, current filename) do NOT.
    // This prevents constant search re-runs while a scan is running.
    let previousStatusSig = "";
    const update = async () => {
      try {
        const list = await call<Progress[]>("jobs");
        if (!alive) return;
        setJobs(list || []);
        // Status signature: only job_id + status pairs (not completed/current/errors counts).
        const statusSig = JSON.stringify(
          (list || []).map((j) => ({ id: j.job_id, status: j.status }))
        );
        if (statusSig !== previousStatusSig) {
          previousStatusSig = statusSig;
          refresh();
        }
        const sources = await call<Source[]>("sources");
        setRoots(sources || []);
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
  }, [refresh]);

  useEffect(() => {
    setOffset(0);
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

  // Re-fetch selected sound only when selectedId changes — NOT on every results update.
  // Previously this fired on results changes (every 1.5s poll), causing two IPC calls to
  // race and momentarily highlight two sounds at once.
  useEffect(() => {
    if (selectedId && isTauri()) {
      call<Sound>("get_sound", { id: selectedId })
        .then((s) => setSelected(s))
        .catch(() => setSelected(null));
    }
  }, [selectedId, setSelected]);

  return {
    info,
    roots,
    setRoots,
    jobs,
    text,
    setText,
    source,
    setSource,
    max,
    setMax,
    offset,
    setOffset,
    results,
    busy,
    revision,
    filters,
    setFilters,
    error,
    setError,
    announcement,
    setAnnouncement,
    addFolder,
    importPath,
    refresh
  };
}
