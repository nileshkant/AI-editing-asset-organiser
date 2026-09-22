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
             try {
                for (const path of paths) await importPath(path);
             } catch (e) {
                setError(String(e));
             }
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
