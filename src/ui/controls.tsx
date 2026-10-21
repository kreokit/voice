import { useRef } from "react";
import { Check } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { ICON_STROKE } from "./Button";
import { nextIndex } from "./logic";

/** A labelled row or stack around any control. */
export function Field({
  label,
  hint,
  inline,
  children,
}: {
  label: string;
  hint?: React.ReactNode;
  /** Label left, control right (inspector rows). */
  inline?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className={inline ? "ui-field ui-field-inline" : "ui-field"}>
      <span className="ui-field-label">{label}</span>
      <div className="ui-field-control">{children}</div>
      {hint && <span className="ui-field-hint">{hint}</span>}
    </div>
  );
}

export function SectionLabel({ children }: { children: React.ReactNode }) {
  return <h3 className="ui-section-label">{children}</h3>;
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      className="ui-switch-row"
      onClick={() => onChange(!checked)}
    >
      <span className="ui-switch-label">{label}</span>
      <span className="ui-switch" aria-hidden>
        <span className="ui-switch-knob" />
      </span>
    </button>
  );
}

export function Checkbox({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      disabled={disabled}
      className="ui-checkbox-row"
      onClick={() => onChange(!checked)}
    >
      <span className="ui-checkbox" aria-hidden>
        {checked && <Check size={12} strokeWidth={3} />}
      </span>
      {label}
    </button>
  );
}

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  icon?: LucideIcon;
  /** Second line, e.g. "fast · 148 MB". */
  hint?: string;
  /** This option can't be chosen (the others still can). */
  disabled?: boolean;
}

/** Radio group drawn as joined buttons; arrow keys move the selection. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
  disabled,
}: {
  value: T;
  options: SegmentOption<T>[];
  onChange: (value: T) => void;
  label: string;
  disabled?: boolean;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const selected = options.findIndex((o) => o.value === value);
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="ui-segmented"
      onKeyDown={(e) => {
        const delta = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
        if (!delta || disabled) return;
        e.preventDefault();
        let to = nextIndex(selected, delta, options.length);
        for (let tries = 1; options[to].disabled && tries < options.length; tries++) {
          to = nextIndex(to, delta, options.length);
        }
        if (options[to].disabled) return;
        onChange(options[to].value);
        refs.current[to]?.focus();
      }}
    >
      {options.map((o, i) => {
        const Icon = o.icon;
        return (
          <button
            key={o.value}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={o.value === value}
            tabIndex={o.value === value || (selected < 0 && i === 0) ? 0 : -1}
            disabled={disabled || o.disabled}
            className="ui-segment"
            onClick={() => onChange(o.value)}
          >
            {Icon && <Icon size={14} strokeWidth={ICON_STROKE} aria-hidden />}
            <span className="ui-segment-text">
              <span>{o.label}</span>
              {o.hint && <span className="ui-segment-hint">{o.hint}</span>}
            </span>
          </button>
        );
      })}
    </div>
  );
}

/** Tab list with arrow-key navigation; the caller renders the selected panel. */
export function Tabs<T extends string>({
  value,
  tabs,
  onChange,
  label,
}: {
  value: T;
  tabs: { value: T; label: string }[];
  onChange: (value: T) => void;
  label: string;
}) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const selected = tabs.findIndex((t) => t.value === value);
  return (
    <div
      role="tablist"
      aria-label={label}
      className="ui-tabs"
      onKeyDown={(e) => {
        const delta = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
        if (!delta) return;
        e.preventDefault();
        const to = nextIndex(selected, delta, tabs.length);
        onChange(tabs[to].value);
        refs.current[to]?.focus();
      }}
    >
      {tabs.map((t, i) => (
        <button
          key={t.value}
          ref={(el) => {
            refs.current[i] = el;
          }}
          type="button"
          role="tab"
          id={`tab-${t.value}`}
          aria-selected={t.value === value}
          aria-controls={`tabpanel-${t.value}`}
          tabIndex={t.value === value ? 0 : -1}
          className="ui-tab"
          onClick={() => onChange(t.value)}
        >
          {t.label}
        </button>
      ))}
    </div>
  );
}

export function TextInput(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={["ui-input", props.className].filter(Boolean).join(" ")} />;
}

export function ProgressBar({ value, label }: { value: number | null; label: string }) {
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value ?? undefined}
      className={value === null ? "ui-progress ui-progress-indeterminate" : "ui-progress"}
    >
      <div className="ui-progress-fill" style={value === null ? undefined : { width: `${value}%` }} />
    </div>
  );
}
