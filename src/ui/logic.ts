/** Pure helpers behind the UI kit (positioning, keyboard navigation, values). */

export interface Rect {
  top: number;
  left: number;
  width: number;
  height: number;
}

export interface Size {
  width: number;
  height: number;
}

/** Popovers keep this far from the window edges and from their anchor. */
export const EDGE = 8;
export const GAP = 4;

/**
 * Where to put a popover of `size` under `anchor` (start = left-aligned, end =
 * right-aligned). Flips above when there is no room below, then stays inside the window.
 */
export function placePopover(
  anchor: Rect,
  size: Size,
  viewport: Size,
  align: "start" | "end" = "start",
): { top: number; left: number } {
  const below = anchor.top + anchor.height + GAP;
  const above = anchor.top - GAP - size.height;
  const fitsBelow = below + size.height <= viewport.height - EDGE;
  const top = fitsBelow || above < EDGE ? below : above;
  const left = align === "start" ? anchor.left : anchor.left + anchor.width - size.width;
  return {
    top: clamp(top, EDGE, Math.max(EDGE, viewport.height - EDGE - size.height)),
    left: clamp(left, EDGE, Math.max(EDGE, viewport.width - EDGE - size.width)),
  };
}

/** A context menu at the pointer, shifted to stay inside the window. */
export function placeAtPoint(point: { x: number; y: number }, size: Size, viewport: Size) {
  const left = point.x + size.width > viewport.width - EDGE ? point.x - size.width : point.x;
  const top = point.y + size.height > viewport.height - EDGE ? point.y - size.height : point.y;
  return {
    top: clamp(top, EDGE, Math.max(EDGE, viewport.height - EDGE - size.height)),
    left: clamp(left, EDGE, Math.max(EDGE, viewport.width - EDGE - size.width)),
  };
}

/**
 * The next enabled index from `current` moving by `delta` (wrapping), for arrow keys in
 * menus, lists and segmented controls. Returns -1 if nothing is enabled.
 */
export function nextIndex(
  current: number,
  delta: number,
  count: number,
  disabled: (i: number) => boolean = () => false,
): number {
  if (count === 0) return -1;
  let i = current;
  for (let tries = 0; tries < count; tries++) {
    i = (((i + delta) % count) + count) % count;
    if (!disabled(i)) return i;
  }
  return -1;
}

export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** Snaps to the step grid starting at `min`, within [min, max]. */
export function snapToStep(value: number, min: number, max: number, step: number): number {
  const snapped = min + Math.round((value - min) / step) * step;
  // Avoid float noise like 0.30000000000000004.
  const decimals = (String(step).split(".")[1] ?? "").length;
  return clamp(Number(snapped.toFixed(decimals)), min, max);
}

/** Slider value at `ratio` (0–1) of the track. */
export function valueAtRatio(ratio: number, min: number, max: number, step: number): number {
  return snapToStep(min + clamp(ratio, 0, 1) * (max - min), min, max, step);
}

/**
 * Slider value after a key press, or null for keys the slider ignores. Arrows move one
 * step (ten with Shift), Page keys ten steps, Home/End jump to the ends.
 */
export function valueAfterKey(
  key: string,
  shift: boolean,
  value: number,
  min: number,
  max: number,
  step: number,
): number | null {
  const big = step * 10;
  const by = shift ? big : step;
  switch (key) {
    case "ArrowRight":
    case "ArrowUp":
      return snapToStep(value + by, min, max, step);
    case "ArrowLeft":
    case "ArrowDown":
      return snapToStep(value - by, min, max, step);
    case "PageUp":
      return snapToStep(value + big, min, max, step);
    case "PageDown":
      return snapToStep(value - big, min, max, step);
    case "Home":
      return min;
    case "End":
      return max;
    default:
      return null;
  }
}

/** "#c8f135", "C8F135", "#cf3" → "#C8F135" / "#CCFF33"; anything else → null. */
export function normalizeHex(text: string): string | null {
  const hex = text.trim().replace(/^#/, "");
  if (/^[0-9a-f]{3}$/i.test(hex)) {
    return ("#" + [...hex].map((c) => c + c).join("")).toUpperCase();
  }
  if (/^[0-9a-f]{6}$/i.test(hex)) return ("#" + hex).toUpperCase();
  return null;
}

/**
 * A typed number for a field shown with one decimal ("12.5", "12,5 %"), clamped to
 * [min, max]; null if it isn't a number.
 */
export function parseNumberField(text: string, min: number, max: number): number | null {
  const n = Number.parseFloat(text.replace(",", ".").replace("%", "").trim());
  if (!Number.isFinite(n)) return null;
  return clamp(Math.round(n * 10) / 10, min, max);
}

/** A colour as hue (0–360), saturation and value (0–1). */
export interface Hsv {
  h: number;
  s: number;
  v: number;
}

/** "#RRGGBB" → HSV (hue 0 for greys). */
export function hexToHsv(hex: string): Hsv {
  const n = Number.parseInt((normalizeHex(hex) ?? "#000000").slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((c) => c / 255);
  const max = Math.max(r, g, b);
  const d = max - Math.min(r, g, b);
  let h = 0;
  if (d > 0) {
    if (max === r) h = ((g - b) / d) % 6;
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h = (h * 60 + 360) % 360;
  }
  return { h, s: max === 0 ? 0 : d / max, v: max };
}

/** HSV → "#RRGGBB". */
export function hsvToHex({ h, s, v }: Hsv): string {
  const f = (k: number) => {
    const x = (k + h / 60) % 6;
    return v - v * s * Math.max(0, Math.min(x, 4 - x, 1));
  };
  const byte = (c: number) => Math.round(clamp(c, 0, 1) * 255).toString(16).padStart(2, "0");
  return ("#" + byte(f(5)) + byte(f(3)) + byte(f(1))).toUpperCase();
}
