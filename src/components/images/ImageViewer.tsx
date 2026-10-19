import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ImagePreview } from '../image/ImagePreview';

export type PreviewBackground = 'checkerboard' | 'light' | 'dark';
export const ZOOM_STEPS = [0.25, 0.5, 1, 2, 4];
const TILE = 512, INSPECT = 2048, MAX_TILES = 48;
/** Same nearest-rounded fit as core `fit`, so tile/backdrop geometry agree. */
const fitScale = (w: number, h: number) => Math.min(1, INSPECT / Math.max(w, h));

type Props = { assetId: string; version: string; width: number; height: number; label: string; background: PreviewBackground; onStale: () => void;
  /** Uncached 1:1 tiles need the original; offline zoom shows the cached preview only. */
  originalAvailable?: boolean };
/**
 * Fit uses the bounded inspect preview. Higher zooms keep that preview as a
 * backdrop and add 1:1 tiles for the visible viewport only, so requests stay
 * bounded and obsolete tiles cancel when they scroll out.
 */
export function ImageViewer({ assetId, version, width, height, label, background, onStale, originalAvailable = true }: Props) {
  const [zoom, setZoom] = useState<'fit' | number>('fit');
  const box = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ left: 0, top: 0, w: 600, h: 400 });
  const centre = useRef<[number, number] | null>(null), drag = useRef<{ x: number; y: number; left: number; top: number } | null>(null);
  useEffect(() => { setZoom('fit'); }, [assetId, version]);
  const scale = zoom === 'fit' ? null : zoom;
  /** Scale at which the whole image fits the visible box (never above 100%). */
  const fitted = () => { const el = box.current; return el && el.clientWidth ? Math.min(1, el.clientWidth / width, el.clientHeight / height) : 1; };
  const zoomTo = (next: 'fit' | number) => {
    const el = box.current;
    // Keep the viewport centre on the same image point across zoom changes.
    if (el && scale) centre.current = [(el.scrollLeft + el.clientWidth / 2) / (width * scale), (el.scrollTop + el.clientHeight / 2) / (height * scale)];
    else centre.current = [0.5, 0.5];
    setZoom(next);
  };
  useLayoutEffect(() => {
    const el = box.current;
    if (!el || !scale) return;
    const [fx, fy] = centre.current ?? [0.5, 0.5];
    el.scrollLeft = fx * width * scale - el.clientWidth / 2;
    el.scrollTop = fy * height * scale - el.clientHeight / 2;
    centre.current = null;
    setView({ left: el.scrollLeft, top: el.scrollTop, w: el.clientWidth, h: el.clientHeight });
  }, [scale, width, height]);
  const step = (direction: 1 | -1) => {
    const current = scale ?? fitted();
    const next = direction > 0 ? ZOOM_STEPS.find(s => s > current + 1e-6) : [...ZOOM_STEPS].reverse().find(s => s < current - 1e-6);
    if (next) zoomTo(next);
  };
  const onKeyDown = (e: React.KeyboardEvent) => {
    const actions: Record<string, () => void> = { '+': () => step(1), '=': () => step(1), '-': () => step(-1), '0': () => zoomTo('fit'), '1': () => zoomTo(1) };
    if (!e.ctrlKey && !e.metaKey && !e.altKey && actions[e.key]) { e.preventDefault(); actions[e.key](); }
  };
  const tiles = [];
  const detail = !!scale && scale > fitScale(width, height) + 1e-6;
  if (detail && originalAvailable) {
    const edge = TILE * scale, cols = Math.ceil(width / TILE), rows = Math.ceil(height / TILE);
    const c0 = Math.max(0, Math.floor(view.left / edge)), c1 = Math.min(cols - 1, Math.floor((view.left + view.w) / edge));
    const r0 = Math.max(0, Math.floor(view.top / edge)), r1 = Math.min(rows - 1, Math.floor((view.top + view.h) / edge));
    if ((c1 - c0 + 1) * (r1 - r0 + 1) <= MAX_TILES) for (let r = r0; r <= r1; ++r) for (let c = c0; c <= c1; ++c) {
      const tw = Math.min(TILE, width - c * TILE), th = Math.min(TILE, height - r * TILE);
      tiles.push(<div key={`${c},${r}`} className="image-viewer-tile" style={{ left: c * edge, top: r * edge, width: tw * scale, height: th * scale }}>
        <ImagePreview assetId={assetId} version={version} tier="tile" tile={[c, r]} label={`${label} tile ${c + 1},${r + 1}`} variant="fill" onStale={onStale} />
      </div>);
    }
  }
  const percent = Math.round((scale ?? fitted()) * 100);
  return <div className="image-viewer-wrap">
    <div className="image-viewer-controls" role="toolbar" aria-label="Zoom">
      <button aria-pressed={zoom === 'fit'} onClick={() => zoomTo('fit')}>Fit</button>
      <button aria-pressed={zoom === 1} onClick={() => zoomTo(1)}>100%</button>
      <button aria-label="Zoom out" disabled={(scale ?? fitted()) <= ZOOM_STEPS[0]} onClick={() => step(-1)}>−</button>
      <button aria-label="Zoom in" disabled={(scale ?? 0) >= ZOOM_STEPS[ZOOM_STEPS.length - 1]} onClick={() => step(1)}>+</button>
      <span role="status" aria-live="polite">{zoom === 'fit' ? `Fit · ${percent}%` : `${percent}%`}</span>
    </div>
    {detail && !originalAvailable && <p className="image-inspector-notice">Full-resolution detail needs the original; showing the cached preview enlarged.</p>}
    <div ref={box} tabIndex={0} role="region" aria-label={`${label} preview. Arrow keys pan; plus, minus, 0 for fit, 1 for 100%.`}
      className={`image-viewer bg-${background}${scale ? ' zoomed' : ''}${scale && scale > 1 ? ' pixelated' : ''}`} onKeyDown={onKeyDown}
      onScroll={e => { const el = e.currentTarget; setView({ left: el.scrollLeft, top: el.scrollTop, w: el.clientWidth, h: el.clientHeight }); }}
      onPointerDown={e => { if (!scale || e.button !== 0) return; drag.current = { x: e.clientX, y: e.clientY, left: e.currentTarget.scrollLeft, top: e.currentTarget.scrollTop }; e.currentTarget.setPointerCapture?.(e.pointerId); }}
      onPointerMove={e => { const d = drag.current; if (!d) return; e.currentTarget.scrollLeft = d.left - (e.clientX - d.x); e.currentTarget.scrollTop = d.top - (e.clientY - d.y); }}
      onPointerUp={() => { drag.current = null; }} onPointerCancel={() => { drag.current = null; }}>
      {scale ? <div className="image-viewer-canvas" style={{ width: width * scale, height: height * scale }}>
        <div className="image-viewer-backdrop"><ImagePreview assetId={assetId} version={version} tier="inspect" label={label} variant="fill" onStale={onStale} /></div>
        {tiles}
      </div> : <ImagePreview assetId={assetId} version={version} tier="inspect" label={label} variant="fill" onStale={onStale} />}
    </div>
  </div>;
}
