/**
 * Timeline maths (from KreoKit Captions): the visible window, zoom and ruler ticks.
 * Pure functions; the timeline component only handles pointers and drawing.
 */

/** The visible part of the timeline: its left edge and zoom. */
export interface View {
  startMs: number;
  msPerPx: number;
}

/** Most zoomed in: 2 ms per pixel (a 100 ms word is 50 px wide). */
export const MIN_MS_PER_PX = 2;
/** Edges snap when within this many pixels. */
export const SNAP_PX = 6;

export const xAt = (ms: number, view: View) => (ms - view.startMs) / view.msPerPx;
export const msAt = (x: number, view: View) => view.startMs + x * view.msPerPx;

/** The whole video in `width` pixels. */
export function fitView(durationMs: number, width: number): View {
  return { startMs: 0, msPerPx: Math.max(MIN_MS_PER_PX, durationMs / Math.max(width, 1)) };
}

/** Keeps the window inside [0, duration], no wider than "fit". */
export function clampView(view: View, durationMs: number, width: number): View {
  const fit = fitView(durationMs, width).msPerPx;
  const msPerPx = Math.min(Math.max(view.msPerPx, MIN_MS_PER_PX), fit);
  const maxStart = Math.max(0, durationMs - width * msPerPx);
  return { msPerPx, startMs: Math.min(Math.max(view.startMs, 0), maxStart) };
}

/** Zooms by `factor` (>1 zooms in), keeping the time under pixel `anchorX` in place. */
export function zoomAt(view: View, factor: number, anchorX: number, durationMs: number, width: number): View {
  const anchorMs = msAt(anchorX, view);
  const msPerPx = view.msPerPx / factor;
  return clampView({ msPerPx, startMs: anchorMs - anchorX * msPerPx }, durationMs, width);
}

/**
 * While playing, keeps the playhead in view: when it passes 90% of the width (or leaves
 * the window), the window pages so the playhead sits at 10%.
 */
export function follow(view: View, ms: number, durationMs: number, width: number): View {
  const x = xAt(ms, view);
  if (x >= 0 && x <= width * 0.9) return view;
  return clampView({ ...view, startMs: ms - width * 0.1 * view.msPerPx }, durationMs, width);
}

const TICK_STEPS = [100, 200, 500, 1000, 2000, 5000, 10_000, 15_000, 30_000, 60_000, 120_000, 300_000, 600_000];

/** Ruler spacing: the smallest step that keeps labels at least `minPx` apart. */
export function tickStep(msPerPx: number, minPx = 72): number {
  return TICK_STEPS.find((step) => step / msPerPx >= minPx) ?? TICK_STEPS[TICK_STEPS.length - 1];
}

/** Ruler label: "0:05", "1:02", "1:00:00"; tenths when the step is under a second. */
export function tickLabel(ms: number, step: number): string {
  const total = Math.round(ms / 100) / 10;
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const sec = step < 1000 ? s.toFixed(1).padStart(4, "0") : String(Math.floor(s)).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}
