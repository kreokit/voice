import { useRef } from "react";
import { clamp, valueAfterKey, valueAtRatio } from "./logic";

/**
 * A flat slider: track with a lime fill and a square handle. Drag, click or use keys.
 * `onChange` follows every move; `onCommit` fires once when a drag ends (or after each
 * key press) with the final value, e.g. for one undo step per drag.
 */
export function Slider({
  value,
  min,
  max,
  step = 1,
  onChange,
  onCommit,
  label,
  format = String,
  disabled,
  origin,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
  onCommit?: (value: number) => void;
  label: string;
  format?: (value: number) => string;
  disabled?: boolean;
  /** Where the fill starts, e.g. 0 for a −/+ slider (default: `min`). */
  origin?: number;
}) {
  const track = useRef<HTMLDivElement>(null);
  /** The value at the start of a drag, and the latest value during it. */
  const drag = useRef<{ start: number; latest: number } | null>(null);
  const toRatio = (v: number) => (max > min ? (clamp(v, min, max) - min) / (max - min) : 0);
  const ratio = toRatio(value);
  const from = toRatio(origin ?? min);

  const fromPointer = (clientX: number) => {
    const r = track.current?.getBoundingClientRect();
    if (!r || r.width === 0) return;
    const next = valueAtRatio((clientX - r.left) / r.width, min, max, step);
    if (drag.current) drag.current.latest = next;
    if (next !== value) onChange(next);
  };

  return (
    <div
      ref={track}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-valuetext={format(value)}
      aria-disabled={disabled}
      className="ui-slider"
      onPointerDown={(e) => {
        if (disabled || e.button !== 0) return;
        e.currentTarget.setPointerCapture(e.pointerId);
        e.currentTarget.focus();
        drag.current = { start: value, latest: value };
        fromPointer(e.clientX);
      }}
      onPointerMove={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) fromPointer(e.clientX);
      }}
      onLostPointerCapture={() => {
        const d = drag.current;
        drag.current = null;
        if (d && d.latest !== d.start) onCommit?.(d.latest);
      }}
      onKeyDown={(e) => {
        if (disabled) return;
        const next = valueAfterKey(e.key, e.shiftKey, value, min, max, step);
        if (next === null) return;
        e.preventDefault();
        if (next !== value) {
          onChange(next);
          onCommit?.(next);
        }
      }}
    >
      <div className="ui-slider-track">
        <div
          className="ui-slider-fill"
          style={{ marginLeft: `${Math.min(from, ratio) * 100}%`, width: `${Math.abs(ratio - from) * 100}%` }}
        />
      </div>
      <div className="ui-slider-thumb" style={{ left: `${ratio * 100}%` }} />
    </div>
  );
}
