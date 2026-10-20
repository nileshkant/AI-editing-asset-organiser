import { memo, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ImagePreview } from '../image/ImagePreview';
import type { ImagePreviewTier, ImageSummary } from '../../types';

export const fileName = (path: string) => path.slice(path.lastIndexOf('/') + 1) || path;
/** Smallest tier covering the rendered cell at the current device pixel ratio. */
export function tierFor(cssPixels: number): ImagePreviewTier {
  const device = cssPixels * (typeof window === 'undefined' ? 1 : window.devicePixelRatio || 1);
  return device <= 128 ? 'small' : device <= 256 ? 'medium' : 'large';
}
const LABEL = 44, GAP = 8;
export const describe = (s: ImageSummary) => {
  const parts = [fileName(s.header.relative_path)];
  if (s.width && s.height) parts.push(`${s.width} × ${s.height}`);
  if (s.header.technical_state !== 'ready') parts.push(s.header.technical_state === 'failed' ? 'technical analysis failed' : 'analysis pending');
  if (s.header.availability !== 'available') parts.push(`original ${s.header.availability}`);
  return parts.join(', ');
};

type Props = {
  /** Data version: windows load asynchronously behind the stable `item` accessor. */
  version: number; total: number; cell: number; item: (index: number) => ImageSummary | undefined; ensure: (first: number, last: number) => void;
  focusIndex: number; setFocusIndex: (index: number) => void; selected: ReadonlyMap<string, ImageSummary>; activeId: string | null;
  onOpen: (index: number, summary: ImageSummary) => void; onToggle: (summary: ImageSummary) => void; onStale: () => void; busy: boolean;
  /** Optional search explanation for a result (SS-058). */
  explain?: (id: string) => string;
};
/**
 * Virtualized ARIA grid: only visible rows (plus one row of overscan) render, so
 * preview requests and DOM nodes stay bounded by the viewport, not the library.
 */
export const ImageGrid = memo(function ImageGrid({ total, cell, item, ensure, focusIndex, setFocusIndex, selected, activeId, onOpen, onToggle, onStale, busy, explain }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ width: 800, height: 600 });
  const [scrollTop, setScrollTop] = useState(0);
  const pendingFocus = useRef(false);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    // A hidden grid (narrow layout with the inspector open) measures 0; keep the last real size.
    const measure = () => { if (el.clientWidth > 0 && el.clientHeight > 0) setSize({ width: el.clientWidth, height: el.clientHeight }); };
    measure();
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(measure);
    observer?.observe(el);
    return () => observer?.disconnect();
  }, []);
  const columns = Math.max(1, Math.floor((size.width - GAP) / (cell + GAP)));
  const rowHeight = cell + LABEL + GAP, rows = Math.ceil(total / columns);
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - 1);
  const last = Math.min(rows - 1, Math.ceil((scrollTop + size.height) / rowHeight) + 1);
  useEffect(() => { if (total) ensure(first * columns, Math.min(total - 1, (last + 1) * columns - 1)); }, [first, last, columns, total, ensure]);
  // Keep the focused cell inside the scrolled viewport, then focus it once rendered.
  const move = useCallback((index: number) => {
    const next = Math.max(0, Math.min(total - 1, index));
    const el = scroller.current, row = Math.floor(next / columns);
    if (el) {
      if (row * rowHeight < el.scrollTop) el.scrollTop = row * rowHeight;
      else if ((row + 1) * rowHeight > el.scrollTop + el.clientHeight) el.scrollTop = (row + 1) * rowHeight - el.clientHeight;
      setScrollTop(el.scrollTop);
    }
    pendingFocus.current = true;
    setFocusIndex(next);
  }, [total, columns, rowHeight, setFocusIndex]);
  // A column change moves cells between row elements, so React re-creates them.
  // If the focused cell is removed that way, focus its replacement for the same item.
  const gridRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const grid = gridRef.current;
    if (!grid) return;
    const onFocusOut = (e: FocusEvent) => {
      const target = e.target as HTMLElement;
      if (e.relatedTarget || !target.dataset.index) return;
      queueMicrotask(() => {
        if (target.isConnected || (document.activeElement && document.activeElement !== document.body)) return;
        grid.querySelector<HTMLElement>(`[data-index="${target.dataset.index}"]`)?.focus({ preventScroll: true });
      });
    };
    grid.addEventListener('focusout', onFocusOut);
    return () => grid.removeEventListener('focusout', onFocusOut);
  }, []);
  useEffect(() => {
    if (!pendingFocus.current) return;
    const target = scroller.current?.querySelector<HTMLElement>(`[data-index="${focusIndex}"]`);
    if (target) { pendingFocus.current = false; target.focus({ preventScroll: true }); }
  });
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!total || e.altKey) return;
    const current = Math.max(0, focusIndex), visibleRows = Math.max(1, Math.floor(size.height / rowHeight));
    const keys: Record<string, number | undefined> = {
      ArrowRight: current + 1, ArrowLeft: current - 1, ArrowDown: current + columns, ArrowUp: current - columns,
      PageDown: current + visibleRows * columns, PageUp: current - visibleRows * columns,
      Home: e.ctrlKey || e.metaKey ? 0 : current - (current % columns),
      End: e.ctrlKey || e.metaKey ? total - 1 : Math.min(total - 1, current - (current % columns) + columns - 1),
    };
    if (e.key in keys) { e.preventDefault(); move(keys[e.key]!); return; }
    const summary = item(current);
    if (!summary) return;
    if (e.key === ' ') { e.preventDefault(); onToggle(summary); }
    else if (e.key === 'Enter') { e.preventDefault(); onOpen(current, summary); }
  };
  const cells = [];
  for (let row = first; row <= last; ++row) {
    const children = [];
    for (let col = 0; col < columns; ++col) {
      const index = row * columns + col;
      if (index >= total) break;
      const s = item(index), isSelected = !!s && selected.has(s.header.id), tabbable = index === Math.max(0, Math.min(focusIndex, total - 1));
      const common = { role: 'gridcell', 'aria-colindex': col + 1, tabIndex: tabbable ? 0 : -1, 'data-index': index, style: { left: GAP + col * (cell + GAP), width: cell } } as const;
      if (!s) { children.push(<div key={index} {...common} className="image-cell placeholder" aria-label={`Loading image ${index + 1}`} onFocus={() => setFocusIndex(index)}><span className="image-cell-thumb" /></div>); continue; }
      const ready = s.header.technical_state === 'ready' && !!s.header.content_hash;
      const reason = explain?.(s.header.id) ?? '';
      children.push(<div key={s.header.id} {...common} aria-selected={isSelected} aria-label={reason ? `${describe(s)}, ${reason}` : describe(s)} title={`${s.header.relative_path}${s.width && s.height ? ` · ${s.width} × ${s.height}` : ''}${reason ? `\n${reason}` : ''}`}
        className={`image-cell${isSelected ? ' selected' : ''}${activeId === s.header.id ? ' active' : ''}`}
        onFocus={() => setFocusIndex(index)}
        onClick={e => { setFocusIndex(index); if (e.metaKey || e.ctrlKey) onToggle(s); else onOpen(index, s); }}>
        <span className="image-cell-thumb" style={{ height: cell }}>
          {ready ? <ImagePreview key={`${s.header.id}:${s.header.content_hash}`} assetId={s.header.id} version={s.header.content_hash!} tier={tierFor(cell)} label={fileName(s.header.relative_path)} variant="fill" onStale={onStale} delayMs={120} />
            : <small className="image-cell-state">{s.header.technical_state === 'failed' ? 'Analysis failed' : 'Analyzing…'}</small>}
          {isSelected && <span className="image-cell-check" aria-hidden="true">✓</span>}
        </span>
        <span className="image-cell-name">{fileName(s.header.relative_path)}</span>
        <span className="image-cell-meta">{s.width && s.height ? `${s.width} × ${s.height}` : ''}{s.header.availability !== 'available' ? ` · ${s.header.availability}` : ''}</span>
      </div>);
    }
    cells.push(<div key={row} role="row" aria-rowindex={row + 1} className="image-row" style={{ top: row * rowHeight, height: rowHeight }}>{children}</div>);
  }
  return <div ref={scroller} className="image-grid-scroller" onScroll={e => setScrollTop(e.currentTarget.scrollTop)}>
    <div ref={gridRef} role="grid" aria-label="Images" aria-rowcount={rows} aria-colcount={columns} aria-multiselectable="true" aria-busy={busy}
      aria-describedby="image-grid-help" className="image-grid" style={{ height: rows * rowHeight }} onKeyDown={onKeyDown}>
      {cells}
    </div>
  </div>;
});
