import { create } from "zustand";
import { Check, CircleAlert, Info } from "lucide-react";
import { ICON_STROKE } from "./Button";

export type ToastKind = "info" | "success" | "error";

interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
}

const DURATION_MS = { info: 3000, success: 3000, error: 6000 };

const useToasts = create<{ items: ToastItem[] }>(() => ({ items: [] }));
let nextId = 1;

/** Shows a short message at the bottom of the window. */
export function toast(message: string, kind: ToastKind = "info") {
  const id = nextId++;
  useToasts.setState((s) => ({ items: [...s.items.slice(-2), { id, message, kind }] }));
  window.setTimeout(
    () => useToasts.setState((s) => ({ items: s.items.filter((t) => t.id !== id) })),
    DURATION_MS[kind],
  );
}

const ICONS = { info: Info, success: Check, error: CircleAlert };

/** Render once near the app root. */
export function ToastRegion() {
  const items = useToasts((s) => s.items);
  return (
    <div className="ui-toasts" role="status" aria-live="polite">
      {items.map((t) => {
        const Icon = ICONS[t.kind];
        return (
          <div key={t.id} className={`ui-toast ui-toast-${t.kind}`}>
            <Icon size={16} strokeWidth={ICON_STROKE} aria-hidden />
            <span>{t.message}</span>
          </div>
        );
      })}
    </div>
  );
}
