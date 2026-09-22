import { useEffect, useRef } from "react";
export function Waveform({
  peaks,
  position = 0,
  onSeek,
}: {
  peaks: [number, number][];
  position?: number;
  onSeek?: (fraction: number) => void;
}) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const draw = () => {
      const width = canvas.clientWidth,
        height = canvas.clientHeight,
        dpr = window.devicePixelRatio || 1;
      canvas.width = Math.round(width * dpr);
      canvas.height = Math.round(height * dpr);
      const c = canvas.getContext("2d");
      if (!c) return;
      c.scale(dpr, dpr);
      c.clearRect(0, 0, width, height);
      c.fillStyle = "#34343d";
      c.fillRect(0, height / 2, width, 1);
      if (!peaks.length) return;
      const bars = Math.max(1, Math.floor(width / 3));
      for (let i = 0; i < bars; i++) {
        const start = Math.floor((i * peaks.length) / bars),
          end = Math.max(
            start + 1,
            Math.floor(((i + 1) * peaks.length) / bars),
          );
        let lo = 0,
          hi = 0;
        for (let j = start; j < Math.min(end, peaks.length); j++) {
          lo = Math.min(lo, peaks[j][0]);
          hi = Math.max(hi, peaks[j][1]);
        }
        c.fillStyle = i / bars <= position ? "#b8f56a" : "#72737e";
        const y = height / 2 - Math.min(1, hi) * height * 0.43;
        const bottom = height / 2 - Math.max(-1, lo) * height * 0.43;
        c.fillRect(
          (i * width) / bars,
          y,
          Math.max(1, width / bars - 1),
          Math.max(1, bottom - y),
        );
      }
      c.fillStyle = "#b8f56a";
      c.fillRect(
        Math.min(width - 1, Math.max(0, position * width)),
        0,
        1,
        height,
      );
    };
    const observer = new ResizeObserver(draw);
    observer.observe(canvas);
    draw();
    return () => observer.disconnect();
  }, [peaks, position]);
  return (
    <canvas
      ref={ref}
      className="waveform"
      role="img"
      aria-label="Measured audio waveform"
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        onSeek?.(Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)));
      }}
    />
  );
}
