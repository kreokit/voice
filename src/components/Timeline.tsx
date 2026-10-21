import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Maximize2, ZoomIn, ZoomOut } from "lucide-react";
import { View, clampView, fitView, follow, msAt, tickLabel, tickStep, xAt, zoomAt } from "../lib/timeline";
import { IconButton } from "../ui";

/** Waveform levels per second (src-tauri/src/preview.rs). */
export const PEAKS_PER_SECOND = 100;

/**
 * The recording on a timeline: ruler, waveform and playhead. Click or drag to seek;
 * scroll to move, ⌘/Ctrl + scroll to zoom (timeline maths from KreoKit Clips).
 */
export function Timeline({
  durationMs,
  peaks,
  currentMs,
  playing,
  onSeek,
}: {
  durationMs: number;
  peaks: Uint8Array | null;
  currentMs: number;
  playing: boolean;
  onSeek: (ms: number) => void;
}) {
  const body = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [view, setView] = useState<View>({ startMs: 0, msPerPx: 50 });
  const fitted = useRef(false);
  const scrubbing = useRef(false);

  useLayoutEffect(() => {
    const el = body.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // The whole recording fits when it opens; later resizes keep the zoom.
  useEffect(() => {
    if (!width) return;
    if (!fitted.current) {
      fitted.current = true;
      setView(fitView(durationMs, width));
    } else setView((v) => clampView(v, durationMs, width));
  }, [width, durationMs]);

  useEffect(() => {
    if (playing && width) setView((v) => follow(v, currentMs, durationMs, width));
  }, [playing, currentMs, width, durationMs]);

  useEffect(() => {
    const el = body.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!width) return;
      e.preventDefault();
      const x = e.clientX - el.getBoundingClientRect().left;
      if (e.ctrlKey || e.metaKey) setView((v) => zoomAt(v, Math.exp(-e.deltaY * 0.01), x, durationMs, width));
      else {
        const delta = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
        setView((v) => clampView({ ...v, startMs: v.startMs + delta * v.msPerPx }, durationMs, width));
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [durationMs, width]);

  const seekToClientX = (clientX: number) => {
    const x = clientX - (body.current?.getBoundingClientRect().left ?? 0);
    onSeek(Math.round(Math.min(Math.max(msAt(x, view), 0), durationMs)));
  };

  const zoomBy = (factor: number) => setView((v) => zoomAt(v, factor, xAt(currentMs, v), durationMs, width));
  const visibleTo = view.startMs + width * view.msPerPx;
  const step = tickStep(view.msPerPx);
  const ticks: number[] = [];
  for (let t = Math.floor(view.startMs / step) * step; t <= visibleTo; t += step) if (t >= 0) ticks.push(t);
  const playheadX = xAt(currentMs, view);

  return (
    <section className="timeline" aria-label="Timeline">
      <div className="timeline-toolbar">
        <IconButton icon={Maximize2} label="Show the whole recording" size="sm" onClick={() => setView(fitView(durationMs, width))} />
        <IconButton icon={ZoomOut} label="Zoom out" shortcut="⌘ scroll" size="sm" onClick={() => zoomBy(1 / 1.5)} />
        <IconButton icon={ZoomIn} label="Zoom in" shortcut="⌘ scroll" size="sm" onClick={() => zoomBy(1.5)} />
        <span className="timeline-hint">Click or drag to move the playhead · scroll to move along</span>
      </div>
      <div
        ref={body}
        className="timeline-body"
        aria-label="Timeline: click to move the playhead"
        onPointerDown={(e) => {
          if (e.button !== 0) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          scrubbing.current = true;
          seekToClientX(e.clientX);
        }}
        onPointerMove={(e) => scrubbing.current && seekToClientX(e.clientX)}
        onPointerUp={() => (scrubbing.current = false)}
        onPointerCancel={() => (scrubbing.current = false)}
      >
        <div className="timeline-ruler">
          {ticks.map((t) => (
            <span key={t} className="timeline-tick" style={{ left: xAt(t, view) }}>
              {tickLabel(t, step)}
            </span>
          ))}
        </div>
        <Waveform peaks={peaks} view={view} width={width} />
        {playheadX >= 0 && playheadX <= width && <div className="timeline-playhead" style={{ left: playheadX }} />}
      </div>
    </section>
  );
}

/** Peak levels drawn mirrored around the middle, one column per pixel. */
function Waveform({ peaks, view, width }: { peaks: Uint8Array | null; view: View; width: number }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [height, setHeight] = useState(0);

  useLayoutEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => setHeight(entry.contentRect.height));
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const el = canvas.current;
    const ctx = el?.getContext("2d");
    if (!el || !ctx || !width || !height) return;
    const dpr = window.devicePixelRatio || 1;
    el.width = Math.round(width * dpr);
    el.height = Math.round(height * dpr);
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, width, height);
    if (!peaks) return;
    ctx.fillStyle = getComputedStyle(el).getPropertyValue("--color-waveform-active").trim();
    const perMs = PEAKS_PER_SECOND / 1000;
    const mid = height / 2;
    for (let x = 0; x < width; x++) {
      const from = Math.floor(msAt(x, view) * perMs);
      const to = Math.max(from + 1, Math.floor(msAt(x + 1, view) * perMs));
      let level = 0;
      for (let i = Math.max(from, 0); i < Math.min(to, peaks.length); i++) level = Math.max(level, peaks[i]);
      if (level === 0) continue;
      const h = Math.max(1, (level / 255) * (height - 8));
      ctx.fillRect(x, mid - h / 2, 1, h);
    }
  }, [peaks, view, width, height]);

  return <canvas ref={canvas} className="timeline-waveform" style={{ width }} aria-hidden />;
}
