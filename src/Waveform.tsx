import { useState, useEffect, useRef, useCallback } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { ZoomIn, ZoomOut, Maximize2, Split, Layers, X, Bookmark, Crosshair } from 'lucide-react';
import { call, duration, type WaveformResponse, type ChannelBucket } from './api';

export interface WaveformProps {
  soundId?: string;
  peaks?: [number, number][];
  duration?: number;
  sampleRate?: number;
  channels?: number;
  playbackPosition?: number;
  isCurrentPlaying?: boolean;
  onSeek?: (seconds: number) => void;
  selection?: { start: number; end: number } | null;
  onSelectionChange?: (selection: { start: number; end: number } | null) => void;
}

export function Waveform({
  soundId,
  peaks = [],
  duration: totalDuration = 1.0,
  sampleRate = 48000,
  channels = 1,
  playbackPosition = 0,
  isCurrentPlaying = false,
  onSeek,
  selection: controlledSelection,
  onSelectionChange,
}: WaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [zoom, setZoom] = useState(1);
  const [panOffset, setPanOffset] = useState(0); // 0.0 to 1.0
  const [stereoSeparate, setStereoSeparate] = useState(channels >= 2);
  const [internalSelection, setInternalSelection] = useState<{ start: number; end: number } | null>(null);
  const [tiles, setTiles] = useState<WaveformResponse | null>(null);
  const [isDragging, setIsDragging] = useState<'create' | 'start' | 'end' | null>(null);
  const [dragAnchor, setDragAnchor] = useState<number>(0);

  const activeSelection = controlledSelection !== undefined ? controlledSelection : internalSelection;
  const updateSelection = useCallback(
    (sel: { start: number; end: number } | null) => {
      if (controlledSelection === undefined) {
        setInternalSelection(sel);
      }
      onSelectionChange?.(sel);
    },
    [controlledSelection, onSelectionChange]
  );

  const safeDuration = Math.max(0.01, totalDuration);
  const visibleDuration = safeDuration / zoom;
  const maxPanStart = Math.max(0, safeDuration - visibleDuration);
  const windowStart = panOffset * maxPanStart;
  const windowEnd = windowStart + visibleDuration;

  const totalFrames = Math.round(safeDuration * sampleRate);
  const startFrame = Math.round(windowStart * sampleRate);
  const endFrame = Math.round(windowEnd * sampleRate);

  // Fetch multiresolution tiles from backend
  useEffect(() => {
    if (!isTauri() || !soundId) {
      setTiles(null);
      return;
    }
    let alive = true;
    call<WaveformResponse>('get_waveform', {
      id: soundId,
      startFrame,
      endFrame,
      maxPoints: 1200,
    })
      .then((res) => {
        if (alive) setTiles(res);
      })
      .catch(() => {
        if (alive) setTiles(null);
      });

    return () => {
      alive = false;
    };
  }, [soundId, startFrame, endFrame]);

  // Zoom controls
  const handleZoomIn = () => setZoom((z) => Math.min(64, z * 2));
  const handleZoomOut = () => {
    setZoom((z) => {
      const next = Math.max(1, z / 2);
      if (next === 1) setPanOffset(0);
      return next;
    });
  };
  const handleZoomFit = () => {
    setZoom(1);
    setPanOffset(0);
  };

  const handleFitSelection = () => {
    if (!activeSelection) return;
    const selSpan = activeSelection.end - activeSelection.start;
    if (selSpan <= 0.001) return;
    const targetZoom = Math.min(64, Math.max(1, safeDuration / selSpan));
    const targetPan = maxPanStart > 0 ? activeSelection.start / maxPanStart : 0;
    setZoom(targetZoom);
    setPanOffset(Math.max(0, Math.min(1, targetPan)));
  };

  // Convert canvas pixel X to time in seconds
  const pixelToSeconds = useCallback(
    (pixelX: number, width: number) => {
      const fraction = Math.max(0, Math.min(1, pixelX / width));
      return windowStart + fraction * visibleDuration;
    },
    [windowStart, visibleDuration]
  );

  // Convert seconds to canvas pixel X
  const secondsToPixel = useCallback(
    (sec: number, width: number) => {
      const fraction = (sec - windowStart) / visibleDuration;
      return fraction * width;
    },
    [windowStart, visibleDuration]
  );

  // Draw canvas
  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    if (width === 0 || height === 0) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, width, height);

    // Background grid
    ctx.fillStyle = '#18181d';
    ctx.fillRect(0, 0, width, height);

    const isSplit = stereoSeparate && channels >= 2;
    const laneCount = isSplit ? 2 : 1;
    const laneHeight = height / laneCount;

    // Centerlines and dividers
    ctx.strokeStyle = '#2d2d38';
    ctx.lineWidth = 1;
    for (let l = 0; l < laneCount; l++) {
      const cy = l * laneHeight + laneHeight / 2;
      ctx.beginPath();
      ctx.moveTo(0, cy);
      ctx.lineTo(width, cy);
      ctx.stroke();
    }
    if (isSplit) {
      ctx.strokeStyle = '#3e3e4a';
      ctx.beginPath();
      ctx.moveTo(0, laneHeight);
      ctx.lineTo(width, laneHeight);
      ctx.stroke();
    }

    // Time ticks / ruler at top
    ctx.fillStyle = '#7a7a88';
    ctx.font = '10px monospace';
    const tickInterval = visibleDuration > 30 ? 10 : visibleDuration > 10 ? 2 : visibleDuration > 2 ? 0.5 : 0.1;
    const firstTick = Math.ceil(windowStart / tickInterval) * tickInterval;
    for (let t = firstTick; t <= windowEnd; t += tickInterval) {
      const x = secondsToPixel(t, width);
      ctx.strokeStyle = '#343440';
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, height);
      ctx.stroke();
      ctx.fillText(`${t.toFixed(t >= 10 ? 1 : 2)}s`, x + 3, 11);
    }

    // Channel lane labels
    if (isSplit) {
      ctx.fillStyle = '#8f909d';
      ctx.font = 'bold 11px sans-serif';
      ctx.fillText('L', 8, 24);
      ctx.fillText('R', 8, laneHeight + 24);
    }

    // Determine buckets to draw
    const channelData: ChannelBucket[][] = [];
    if (tiles && tiles.channels.length > 0) {
      if (isSplit) {
        channelData.push(tiles.channels[0] || []);
        channelData.push(tiles.channels[1] || tiles.channels[0] || []);
      } else {
        // Combined mode: merge channels
        const ch0 = tiles.channels[0] || [];
        const ch1 = tiles.channels[1] || [];
        const len = Math.max(ch0.length, ch1.length);
        const combined: ChannelBucket[] = [];
        for (let i = 0; i < len; i++) {
          const b0 = ch0[i] || { min: 0, max: 0, rms: 0 };
          const b1 = ch1[i] || b0;
          combined.push({
            min: Math.min(b0.min, b1.min),
            max: Math.max(b0.max, b1.max),
            rms: Math.sqrt(((b0.rms || 0) ** 2 + (b1.rms || 0) ** 2) / 2),
          });
        }
        channelData.push(combined);
      }
    } else if (peaks.length > 0) {
      // Fallback from profile peaks
      const fallbackBuckets: ChannelBucket[] = peaks.map(([min, max]) => ({
        min,
        max,
        rms: (Math.abs(min) + Math.abs(max)) / 4,
      }));
      channelData.push(fallbackBuckets);
      if (isSplit) {
        channelData.push(fallbackBuckets);
      }
    }

    // Draw waveform bars per lane
    for (let l = 0; l < laneCount; l++) {
      const buckets = channelData[l] || [];
      if (!buckets.length) continue;

      const laneY = l * laneHeight;
      const centerY = laneY + laneHeight / 2;
      const halfLane = laneHeight * 0.44;

      const bucketWidth = width / buckets.length;
      for (let i = 0; i < buckets.length; i++) {
        const b = buckets[i];
        const x = i * bucketWidth;

        // Draw Peak envelope bar
        const topY = centerY - Math.min(1, Math.max(0, b.max)) * halfLane;
        const bottomY = centerY - Math.max(-1, Math.min(0, b.min)) * halfLane;
        const barHeight = Math.max(1, bottomY - topY);

        ctx.fillStyle = '#7a9657';
        ctx.fillRect(x, topY, Math.max(1, bucketWidth - 0.5), barHeight);

        // Draw RMS center energy bar
        const rmsHeight = Math.min(halfLane, b.rms * halfLane);
        ctx.fillStyle = '#b8f56a';
        ctx.fillRect(x, centerY - rmsHeight, Math.max(1, bucketWidth - 0.5), rmsHeight * 2);
      }
    }

    // Shaded selection region
    if (activeSelection) {
      const selStart = Math.max(windowStart, activeSelection.start);
      const selEnd = Math.min(windowEnd, activeSelection.end);
      if (selEnd > selStart) {
        const x1 = secondsToPixel(selStart, width);
        const x2 = secondsToPixel(selEnd, width);

        ctx.fillStyle = 'rgba(184, 245, 106, 0.16)';
        ctx.fillRect(x1, 0, x2 - x1, height);

        // Handles
        ctx.strokeStyle = '#b8f56a';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(x1, 0);
        ctx.lineTo(x1, height);
        ctx.moveTo(x2, 0);
        ctx.lineTo(x2, height);
        ctx.stroke();

        // Handle tags
        ctx.fillStyle = '#b8f56a';
        ctx.fillRect(x1 - 2, 0, 4, 14);
        ctx.fillRect(x2 - 2, 0, 4, 14);
      }
    }

    // Playback cursor
    if (playbackPosition >= windowStart && playbackPosition <= windowEnd) {
      const cursorX = secondsToPixel(playbackPosition, width);
      ctx.strokeStyle = '#ffffff';
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(cursorX, 0);
      ctx.lineTo(cursorX, height);
      ctx.stroke();

      // Playhead indicator triangle at top
      ctx.fillStyle = '#b8f56a';
      ctx.beginPath();
      ctx.moveTo(cursorX - 5, 0);
      ctx.lineTo(cursorX + 5, 0);
      ctx.lineTo(cursorX, 7);
      ctx.closePath();
      ctx.fill();
    }
  }, [
    stereoSeparate,
    channels,
    visibleDuration,
    windowStart,
    windowEnd,
    secondsToPixel,
    tiles,
    peaks,
    activeSelection,
    playbackPosition,
  ]);

  useEffect(() => {
    draw();
  }, [draw]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [draw]);

  // Mouse interaction: seek, drag-to-select, and handle dragging
  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const clickX = e.clientX - rect.left;
    const clickTime = pixelToSeconds(clickX, rect.width);

    // Check if clicked near selection handles
    if (activeSelection) {
      const startX = secondsToPixel(activeSelection.start, rect.width);
      const endX = secondsToPixel(activeSelection.end, rect.width);
      if (Math.abs(clickX - startX) <= 8) {
        setIsDragging('start');
        return;
      }
      if (Math.abs(clickX - endX) <= 8) {
        setIsDragging('end');
        return;
      }
    }

    // Otherwise start new selection or seek
    if (e.shiftKey) {
      setIsDragging('create');
      setDragAnchor(clickTime);
      updateSelection({ start: clickTime, end: clickTime });
    } else {
      // Seek on simple click
      onSeek?.(clickTime);
      setIsDragging('create');
      setDragAnchor(clickTime);
    }
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas || !isDragging) return;
    const rect = canvas.getBoundingClientRect();
    const curTime = pixelToSeconds(e.clientX - rect.left, rect.width);

    if (isDragging === 'create') {
      const start = Math.min(dragAnchor, curTime);
      const end = Math.max(dragAnchor, curTime);
      if (end - start > 0.01) {
        updateSelection({ start, end });
      }
    } else if (isDragging === 'start' && activeSelection) {
      const nextStart = Math.min(activeSelection.end - 0.001, Math.max(0, curTime));
      updateSelection({ start: nextStart, end: activeSelection.end });
    } else if (isDragging === 'end' && activeSelection) {
      const nextEnd = Math.max(activeSelection.start + 0.001, Math.min(safeDuration, curTime));
      updateSelection({ start: activeSelection.start, end: nextEnd });
    }
  };

  const handleMouseUp = () => {
    setIsDragging(null);
  };

  // Keyboard nudging for selection
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (!activeSelection) return;
    const step = e.shiftKey ? 0.1 : 0.01;
    if (e.key === 'ArrowLeft') {
      e.preventDefault();
      const start = Math.max(0, activeSelection.start - step);
      const end = Math.max(start + 0.001, activeSelection.end - step);
      updateSelection({ start, end });
    } else if (e.key === 'ArrowRight') {
      e.preventDefault();
      const end = Math.min(safeDuration, activeSelection.end + step);
      const start = Math.min(end - 0.001, activeSelection.start + step);
      updateSelection({ start, end });
    } else if (e.key === 'i' || e.key === 'I') {
      e.preventDefault();
      updateSelection({ start: playbackPosition, end: Math.max(playbackPosition + 0.01, activeSelection.end) });
    } else if (e.key === 'o' || e.key === 'O') {
      e.preventDefault();
      updateSelection({ start: Math.min(playbackPosition - 0.01, activeSelection.start), end: playbackPosition });
    }
  };

  return (
    <div
      className="waveform-workstation"
      role="region"
      aria-label="Audio waveform workstation"
      tabIndex={0}
      onKeyDown={handleKeyDown}
    >
      {/* Waveform Controls Toolbar */}
      <div className="waveform-toolbar" role="toolbar" aria-label="Waveform controls">
        <div className="waveform-group">
          {channels >= 2 && (
            <button
              className={`icon-button ${stereoSeparate ? 'chosen' : ''}`}
              title={stereoSeparate ? 'Combined waveform view' : 'Split stereo channels'}
              aria-label={stereoSeparate ? 'Combined waveform view' : 'Split stereo channels'}
              aria-pressed={stereoSeparate}
              onClick={() => setStereoSeparate(!stereoSeparate)}
            >
              {stereoSeparate ? <Split size={14} aria-hidden="true" /> : <Layers size={14} aria-hidden="true" />}
            </button>
          )}
          <span className="waveform-stat">
            {channels === 1 ? 'Mono' : channels === 2 ? 'Stereo' : `${channels} Ch`} · {(sampleRate / 1000).toFixed(1)} kHz
          </span>
        </div>

        <div className="waveform-group">
          <button
            className="icon-button"
            title="Zoom out"
            aria-label="Zoom out"
            disabled={zoom <= 1}
            onClick={handleZoomOut}
          >
            <ZoomOut size={14} aria-hidden="true" />
          </button>
          <span className="zoom-badge" aria-label={`Zoom level ${zoom}x`}>
            {zoom}x
          </span>
          <button
            className="icon-button"
            title="Zoom in"
            aria-label="Zoom in"
            disabled={zoom >= 64}
            onClick={handleZoomIn}
          >
            <ZoomIn size={14} aria-hidden="true" />
          </button>
          <button
            className="icon-button"
            title="Fit to window"
            aria-label="Fit to window"
            onClick={handleZoomFit}
          >
            <Maximize2 size={14} aria-hidden="true" />
          </button>
        </div>

        {activeSelection && (
          <div className="waveform-group">
            <button
              className="icon-button"
              title="Fit to selection"
              aria-label="Fit to selection"
              onClick={handleFitSelection}
            >
              <Bookmark size={14} aria-hidden="true" />
            </button>
            <button
              className="icon-button"
              title="Clear selection"
              aria-label="Clear selection"
              onClick={() => updateSelection(null)}
            >
              <X size={14} aria-hidden="true" />
            </button>
          </div>
        )}
      </div>

      {/* Main Canvas Viewport */}
      <div className="waveform-canvas-container">
        <canvas
          ref={canvasRef}
          className="waveform-canvas"
          role="img"
          aria-label={`Audio waveform with duration ${duration(safeDuration)}`}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseUp}
        />
      </div>

      {/* Horizontal Pan Scrollbar when Zoomed In */}
      {zoom > 1 && (
        <div className="waveform-pan-bar" role="group" aria-label="Waveform pan navigation">
          <input
            type="range"
            min="0"
            max="1"
            step="0.005"
            value={panOffset}
            aria-label="Pan offset"
            aria-valuetext={`${duration(windowStart)} to ${duration(windowEnd)}`}
            onChange={(e) => setPanOffset(Number(e.target.value))}
            className="waveform-pan-slider"
          />
        </div>
      )}

      {/* Accessible Numeric Selection Panel */}
      <div className="waveform-selection-panel" role="group" aria-label="Selection boundaries">
        {activeSelection ? (
          <>
            <div className="numeric-input-group">
              <label>
                Start
                <input
                  type="number"
                  step="0.01"
                  min="0"
                  max={activeSelection.end}
                  value={activeSelection.start.toFixed(3)}
                  aria-label="Selection start in seconds"
                  onChange={(e) => {
                    const next = Math.max(0, Math.min(activeSelection.end - 0.001, Number(e.target.value)));
                    updateSelection({ start: next, end: activeSelection.end });
                  }}
                />
              </label>
              <label>
                End
                <input
                  type="number"
                  step="0.01"
                  min={activeSelection.start}
                  max={safeDuration}
                  value={activeSelection.end.toFixed(3)}
                  aria-label="Selection end in seconds"
                  onChange={(e) => {
                    const next = Math.min(safeDuration, Math.max(activeSelection.start + 0.001, Number(e.target.value)));
                    updateSelection({ start: activeSelection.start, end: next });
                  }}
                />
              </label>
              <label>
                Duration
                <input
                  type="number"
                  step="0.01"
                  readOnly
                  aria-label="Selection duration in seconds"
                  value={(activeSelection.end - activeSelection.start).toFixed(3)}
                />
              </label>
            </div>
            <div className="selection-actions">
              <button
                className="compact-button"
                title="Set selection start at playhead cursor"
                onClick={() =>
                  updateSelection({
                    start: Math.max(0, Math.min(activeSelection.end - 0.01, playbackPosition)),
                    end: activeSelection.end,
                  })
                }
              >
                <Crosshair size={12} aria-hidden="true" /> Start at Playhead
              </button>
              <button
                className="compact-button"
                title="Set selection end at playhead cursor"
                onClick={() =>
                  updateSelection({
                    start: activeSelection.start,
                    end: Math.min(safeDuration, Math.max(activeSelection.start + 0.01, playbackPosition)),
                  })
                }
              >
                <Crosshair size={12} aria-hidden="true" /> End at Playhead
              </button>
            </div>
          </>
        ) : (
          <div className="selection-prompt">
            <small className="muted">Shift+drag or click on the waveform to create a selection region</small>
            <button
              className="compact-button"
              onClick={() => updateSelection({ start: 0, end: safeDuration })}
            >
              Select All
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
