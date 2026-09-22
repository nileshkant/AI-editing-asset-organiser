import React, { useRef, useEffect, useCallback } from 'react';

interface PitchOverlayProps {
  /** Peak/RMS waveform buckets to derive spectral energy approximation. */
  peaks: [number, number][];
  /** Total duration in seconds. */
  duration: number;
  /** Current playback position in seconds. */
  playbackPosition: number;
  /** Whether audio is currently playing. */
  isPlaying: boolean;
  /** Width of the parent container (for canvas sizing). */
  width: number;
  /** Height of the parent container. */
  height: number;
  /** Visible time window start. */
  windowStart: number;
  /** Visible time window end. */
  windowEnd: number;
}

/**
 * Renders a semi-transparent spectral energy curve overlay on the waveform.
 *
 * Uses peak-to-RMS ratio (crest factor) as an approximation of spectral
 * brightness. High crest factor = transient/percussive, low = tonal/sustained.
 * This provides visual feedback about tonal vs. noise-like content.
 */
export function PitchOverlay({
  peaks,
  duration: totalDuration,
  playbackPosition,
  isPlaying,
  width,
  height,
  windowStart,
  windowEnd,
}: PitchOverlayProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || !peaks.length || width === 0 || height === 0) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, width, height);

    const safeDuration = Math.max(0.01, totalDuration);
    const visibleDuration = windowEnd - windowStart;

    // Map peaks to visible window
    const startIdx = Math.floor((windowStart / safeDuration) * peaks.length);
    const endIdx = Math.ceil((windowEnd / safeDuration) * peaks.length);
    const visiblePeaks = peaks.slice(
      Math.max(0, startIdx),
      Math.min(peaks.length, endIdx),
    );

    if (!visiblePeaks.length) return;

    // Compute spectral energy approximation per bucket
    // Using crest factor: peak / RMS ratio
    // Higher values = more transient, lower = more tonal
    const energyValues = visiblePeaks.map(([min, max]) => {
      const peakAbs = Math.max(Math.abs(min), Math.abs(max));
      const rmsApprox = (Math.abs(min) + Math.abs(max)) / 4;
      if (rmsApprox < 0.001) return 0;
      // Normalize crest factor to 0-1 range (typical range: 1-6)
      const crestFactor = peakAbs / rmsApprox;
      return Math.min(1, Math.max(0, 1 - (crestFactor - 1) / 5));
    });

    // Smooth the values with a simple moving average
    const smoothed: number[] = [];
    const smoothWindow = Math.max(1, Math.floor(energyValues.length / 80));
    for (let i = 0; i < energyValues.length; i++) {
      let sum = 0;
      let count = 0;
      for (
        let j = Math.max(0, i - smoothWindow);
        j <= Math.min(energyValues.length - 1, i + smoothWindow);
        j++
      ) {
        sum += energyValues[j];
        count++;
      }
      smoothed.push(sum / count);
    }

    // Draw the energy curve
    const gradient = ctx.createLinearGradient(0, 0, 0, height);
    gradient.addColorStop(0, 'rgba(184, 245, 106, 0.25)');
    gradient.addColorStop(0.5, 'rgba(132, 201, 202, 0.15)');
    gradient.addColorStop(1, 'rgba(184, 245, 106, 0.05)');

    ctx.beginPath();
    ctx.moveTo(0, height);

    for (let i = 0; i < smoothed.length; i++) {
      const x = (i / smoothed.length) * width;
      const y = height - smoothed[i] * height * 0.6;
      if (i === 0) {
        ctx.lineTo(x, y);
      } else {
        // Smooth curve using quadratic bezier
        const prevX = ((i - 1) / smoothed.length) * width;
        const cpX = (prevX + x) / 2;
        ctx.quadraticCurveTo(cpX, y, x, y);
      }
    }

    ctx.lineTo(width, height);
    ctx.closePath();
    ctx.fillStyle = gradient;
    ctx.fill();

    // Draw a subtle top edge line
    ctx.beginPath();
    for (let i = 0; i < smoothed.length; i++) {
      const x = (i / smoothed.length) * width;
      const y = height - smoothed[i] * height * 0.6;
      if (i === 0) {
        ctx.moveTo(x, y);
      } else {
        const prevX = ((i - 1) / smoothed.length) * width;
        const cpX = (prevX + x) / 2;
        ctx.quadraticCurveTo(cpX, y, x, y);
      }
    }
    ctx.strokeStyle = 'rgba(184, 245, 106, 0.35)';
    ctx.lineWidth = 1.5;
    ctx.stroke();

    // Draw playback position energy indicator during playback
    if (isPlaying && playbackPosition >= windowStart && playbackPosition <= windowEnd) {
      const playFraction = (playbackPosition - windowStart) / visibleDuration;
      const playIdx = Math.floor(playFraction * smoothed.length);
      if (playIdx >= 0 && playIdx < smoothed.length) {
        const px = playFraction * width;
        const py = height - smoothed[playIdx] * height * 0.6;

        // Glowing dot at playhead
        ctx.beginPath();
        ctx.arc(px, py, 4, 0, Math.PI * 2);
        ctx.fillStyle = 'rgba(184, 245, 106, 0.8)';
        ctx.fill();

        // Glow effect
        ctx.beginPath();
        ctx.arc(px, py, 8, 0, Math.PI * 2);
        ctx.fillStyle = 'rgba(184, 245, 106, 0.2)';
        ctx.fill();
      }
    }
  }, [peaks, totalDuration, playbackPosition, isPlaying, width, height, windowStart, windowEnd]);

  useEffect(() => {
    draw();
  }, [draw]);

  return (
    <canvas
      ref={canvasRef}
      style={{
        position: 'absolute',
        inset: 0,
        width: '100%',
        height: '100%',
        pointerEvents: 'none',
      }}
      aria-hidden="true"
    />
  );
}
