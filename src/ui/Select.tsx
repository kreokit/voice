import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import { ICON_STROKE } from "./Button";
import { nextIndex } from "./logic";
import { Popover } from "./Popover";

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  /** Small secondary text on the right (e.g. "148 MB"). */
  hint?: string;
  /** Render the label in this font (font pickers). */
  fontFamily?: string;
}

/** A flat replacement for <select>: a button that opens a listbox. */
export function Select<T extends string>({
  value,
  options,
  onChange,
  label,
  disabled,
  className,
}: {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  /** Accessible name (visible labels are rendered by Field). */
  label: string;
  disabled?: boolean;
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const anchor = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const id = useId();
  const selected = options.findIndex((o) => o.value === value);
  const current = options[selected];

  const close = useCallback(() => {
    setOpen(false);
    anchor.current?.focus();
  }, []);
  const openAt = (index: number) => {
    setActive(index < 0 ? 0 : index);
    setOpen(true);
  };
  const choose = (index: number) => {
    const option = options[index];
    if (option) onChange(option.value);
    close();
  };

  useEffect(() => {
    if (open) list.current?.focus();
  }, [open]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  return (
    <>
      <button
        ref={anchor}
        type="button"
        className={["ui-select", className].filter(Boolean).join(" ")}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={`${label}: ${current?.label ?? ""}`}
        disabled={disabled}
        onClick={() => (open ? close() : openAt(selected))}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            openAt(selected);
          }
        }}
      >
        <span
          className="ui-select-value"
          style={current?.fontFamily ? { fontFamily: current.fontFamily, fontFeatureSettings: "normal" } : undefined}
        >
          {current?.label}
        </span>
        <ChevronDown size={14} strokeWidth={ICON_STROKE} aria-hidden />
      </button>
      <Popover open={open} onClose={close} anchor={anchor} matchWidth>
        <div
          ref={list}
          role="listbox"
          tabIndex={-1}
          aria-label={label}
          aria-activedescendant={`${id}-${active}`}
          className="ui-listbox"
          onKeyDown={(e) => {
            const move = (to: number) => {
              e.preventDefault();
              setActive(to);
            };
            switch (e.key) {
              case "ArrowDown":
                return move(nextIndex(active, 1, options.length));
              case "ArrowUp":
                return move(nextIndex(active, -1, options.length));
              case "Home":
                return move(0);
              case "End":
                return move(options.length - 1);
              case "Enter":
              case " ":
                e.preventDefault();
                return choose(active);
              case "Tab":
                e.preventDefault();
                return close();
            }
          }}
        >
          {options.map((o, i) => (
            <div
              key={o.value}
              id={`${id}-${i}`}
              data-index={i}
              role="option"
              aria-selected={o.value === value}
              className={i === active ? "ui-option ui-option-active" : "ui-option"}
              onPointerMove={() => setActive(i)}
              onClick={() => choose(i)}
            >
              <span className="ui-option-check">
                {o.value === value && <Check size={14} strokeWidth={2} aria-hidden />}
              </span>
              <span
                className="ui-option-label"
                style={o.fontFamily ? { fontFamily: o.fontFamily, fontFeatureSettings: "normal" } : undefined}
              >
                {o.label}
              </span>
              {o.hint && <span className="ui-option-hint">{o.hint}</span>}
            </div>
          ))}
        </div>
      </Popover>
    </>
  );
}
