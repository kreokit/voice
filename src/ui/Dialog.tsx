import { useEffect, useId, useRef } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { IconButton } from "./Button";

const FOCUSABLE =
  'button:not([disabled]), [href], input:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Modal dialog over a solid 70% backdrop. Focus moves in (to `initialFocus` or the first
 * control), Tab stays inside, Escape closes (unless `dismissible` is false), and focus
 * returns to where it was.
 */
export function Dialog({
  title,
  onClose,
  dismissible = true,
  initialFocus,
  footer,
  width = 440,
  children,
}: {
  title: string;
  onClose: () => void;
  dismissible?: boolean;
  initialFocus?: React.RefObject<HTMLElement | null>;
  footer?: React.ReactNode;
  width?: number;
  children: React.ReactNode;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const titleId = useId();

  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const first = initialFocus?.current ?? panel.current?.querySelector<HTMLElement>(FOCUSABLE);
    (first ?? panel.current)?.focus();
    return () => previous?.focus();
    // Run once per opening.
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape" && dismissible) {
      e.stopPropagation();
      onClose();
    }
    if (e.key !== "Tab" || !panel.current) return;
    const items = [...panel.current.querySelectorAll<HTMLElement>(FOCUSABLE)];
    if (items.length === 0) return;
    const first = items[0];
    const last = items[items.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div className="ui-backdrop" onPointerDown={(e) => e.target === e.currentTarget && dismissible && onClose()}>
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="ui-dialog"
        style={{ width }}
        onKeyDown={onKeyDown}
      >
        <header className="ui-dialog-header">
          <h2 id={titleId}>{title}</h2>
          {dismissible && <IconButton icon={X} label="Close" size="sm" onClick={onClose} />}
        </header>
        <div className="ui-dialog-body">{children}</div>
        {footer && <footer className="ui-dialog-footer">{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}
