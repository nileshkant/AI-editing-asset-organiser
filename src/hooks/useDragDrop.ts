import { useState, useEffect } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';

export function useDragDrop(importPath: (path: string) => Promise<void>, setError: (e: string) => void) {
  const [dropping, setDropping] = useState(false);

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
          void (async () => {
             if (paths.length > 256) {setError("Drop up to 256 paths at a time");return;}
             const errors: string[] = [];
             for (const path of [...new Set(paths)]) {
                if (disposed) break;
                try {await importPath(path);} catch (e) {errors.push(`${path}: ${String(e)}`);}
             }
             if (!disposed) setError(errors.join("\n"));
          })();
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
  }, [importPath, setError]);

  return { dropping };
}
