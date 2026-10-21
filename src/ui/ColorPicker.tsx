import { useCallback, useEffect, useRef, useState } from "react";
import { Hsv, clamp, hexToHsv, hsvToHex, normalizeHex } from "./logic";
import { Popover } from "./Popover";

/** Swatches offered in every colour picker: neutrals, the brand lime, and caption favourites. */
export const SWATCHES = [
  "#FFFFFF",
  "#A1A1AA",
  "#26262B",
  "#000000",
  "#C8F135",
  "#FFD400",
  "#FF8A1F",
  "#FF5A5F",
  "#FF5FD2",
  "#9B6BFF",
  "#3D8BFF",
  "#3DE0FF",
];

/**
 * Drags on `el` report the pointer as 0–1 ratios of its box; returns pointer handlers.
 * `onEnd` fires once when the drag ends.
 */
function usePointerArea(onMove: (x: number, y: number) => void, onEnd: () => void) {
  const at = (e: React.PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    onMove(clamp((e.clientX - r.left) / r.width, 0, 1), clamp((e.clientY - r.top) / r.height, 0, 1));
  };
  return {
    onPointerDown: (e: React.PointerEvent<HTMLDivElement>) => {
      if (e.button !== 0) return;
      e.currentTarget.setPointerCapture(e.pointerId);
      e.currentTarget.focus();
      at(e);
    },
    onPointerMove: (e: React.PointerEvent<HTMLDivElement>) => {
      if (e.currentTarget.hasPointerCapture(e.pointerId)) at(e);
    },
    onLostPointerCapture: onEnd,
  };
}

/**
 * A colour swatch button that opens a flat picker: a saturation/brightness area, a hue
 * bar, swatches and a hex field. `onChange` follows every move; `onCommit` fires once
 * per drag, key press, swatch or typed hex (e.g. one undo step).
 */
export function ColorPicker({
  value,
  onChange,
  onCommit,
  label,
  swatches = SWATCHES,
  disabled,
}: {
  value: string;
  onChange: (hex: string) => void;
  onCommit?: (hex: string) => void;
  label: string;
  swatches?: string[];
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(value);
  // Kept separately so the hue survives greys and black (which have no hue).
  const [hsv, setHsv] = useState<Hsv>(() => hexToHsv(value));
  const latest = useRef(value);
  const anchor = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => {
    setOpen(false);
    anchor.current?.focus();
  }, []);
  useEffect(() => setDraft(value), [value, open]);
  useEffect(() => {
    latest.current = value;
    setHsv((current) => (hsvToHex(current) === value ? current : hexToHsv(value)));
  }, [value]);

  const change = (next: Hsv) => {
    setHsv(next);
    const hex = hsvToHex(next);
    latest.current = hex;
    if (hex !== value) onChange(hex);
  };
  const commit = (hex = latest.current) => onCommit?.(hex);
  const pick = (hex: string) => {
    setHsv(hexToHsv(hex));
    latest.current = hex;
    onChange(hex);
    commit(hex);
  };

  const area = usePointerArea((x, y) => change({ h: hsv.h, s: x, v: 1 - y }), () => commit());
  const hue = usePointerArea((x) => change({ ...hsv, h: Math.min(x * 360, 359.9) }), () => commit());
  const keys = (e: React.KeyboardEvent, step: (by: number, vertical: boolean) => Hsv | null) => {
    const by = e.shiftKey ? 10 : 1;
    const moves: Record<string, [number, boolean]> = {
      ArrowLeft: [-by, false],
      ArrowRight: [by, false],
      ArrowUp: [by, true],
      ArrowDown: [-by, true],
    };
    const move = moves[e.key];
    const next = move && step(...move);
    if (!next) return;
    e.preventDefault();
    change(next);
    commit(hsvToHex(next));
  };

  const commitDraft = () => {
    const hex = normalizeHex(draft);
    if (hex) pick(hex);
    else setDraft(value);
  };

  return (
    <>
      <button
        ref={anchor}
        type="button"
        className="ui-color"
        aria-label={`${label}: ${value}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="ui-color-chip" style={{ background: value }} />
        <span className="ui-color-hex tabular">{value}</span>
      </button>
      <Popover open={open} onClose={close} anchor={anchor}>
        <div role="dialog" aria-label={label} className="ui-color-panel">
          <div
            className="ui-color-area"
            role="slider"
            tabIndex={0}
            aria-label="Saturation and brightness"
            aria-valuetext={`Saturation ${Math.round(hsv.s * 100)}%, brightness ${Math.round(hsv.v * 100)}%`}
            style={{ backgroundColor: hsvToHex({ h: hsv.h, s: 1, v: 1 }) }}
            {...area}
            onKeyDown={(e) =>
              keys(e, (by, vertical) =>
                vertical
                  ? { ...hsv, v: clamp(hsv.v + by / 100, 0, 1) }
                  : { ...hsv, s: clamp(hsv.s + by / 100, 0, 1) },
              )
            }
          >
            <span className="ui-color-area-handle" style={{ left: `${hsv.s * 100}%`, top: `${(1 - hsv.v) * 100}%` }} />
          </div>
          <div
            className="ui-color-hue"
            role="slider"
            tabIndex={0}
            aria-label="Hue"
            aria-valuemin={0}
            aria-valuemax={360}
            aria-valuenow={Math.round(hsv.h)}
            {...hue}
            onKeyDown={(e) => keys(e, (by, vertical) => (vertical ? null : { ...hsv, h: (hsv.h + by + 360) % 360 }))}
          >
            <span className="ui-color-hue-handle" style={{ left: `${(hsv.h / 360) * 100}%` }} />
          </div>
          <div className="ui-swatches" role="group" aria-label="Swatches">
            {swatches.map((hex) => (
              <button
                key={hex}
                type="button"
                className="ui-swatch"
                style={{ background: hex }}
                aria-label={hex}
                aria-pressed={hex === value}
                onClick={() => {
                  pick(hex);
                  close();
                }}
              />
            ))}
          </div>
          <label className="ui-color-input">
            <span>Hex</span>
            <input
              className="ui-input tabular"
              value={draft}
              spellCheck={false}
              maxLength={7}
              aria-invalid={normalizeHex(draft) === null}
              onChange={(e) => setDraft(e.target.value)}
              onBlur={commitDraft}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  commitDraft();
                  close();
                }
              }}
            />
          </label>
        </div>
      </Popover>
    </>
  );
}
