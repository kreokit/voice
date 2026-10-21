import { cloneElement, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { EDGE, GAP, clamp } from "./logic";

const DELAY_MS = 450;

/**
 * Shows `label` (and an optional shortcut) above the child after a short hover or on
 * keyboard focus. The child must accept ref and mouse/focus handlers.
 */
export function Tooltip({
  label,
  shortcut,
  children,
}: {
  label: string;
  shortcut?: string;
  children: React.ReactElement<React.HTMLAttributes<HTMLElement> & { ref?: React.Ref<HTMLElement> }>;
}) {
  const anchor = useRef<HTMLElement | null>(null);
  const tip = useRef<HTMLDivElement>(null);
  const timer = useRef<number | undefined>(undefined);
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);

  const show = (delay: number) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setOpen(true), delay);
  };
  const hide = () => {
    window.clearTimeout(timer.current);
    setOpen(false);
    setPos(null);
  };
  useEffect(() => () => window.clearTimeout(timer.current), []);

  useLayoutEffect(() => {
    if (!open || !anchor.current || !tip.current) return;
    const r = anchor.current.getBoundingClientRect();
    const w = tip.current.offsetWidth;
    const h = tip.current.offsetHeight;
    const above = r.top - GAP - h;
    setPos({
      top: above >= EDGE ? above : r.bottom + GAP,
      left: clamp(r.left + r.width / 2 - w / 2, EDGE, window.innerWidth - EDGE - w),
    });
  }, [open]);

  // Keep the child's own ref working (React 19 passes it as a prop).
  const childRef = (children.props as { ref?: React.Ref<HTMLElement> }).ref;
  const child = cloneElement(children, {
    ref: (el: HTMLElement | null) => {
      anchor.current = el;
      if (typeof childRef === "function") childRef(el);
      else if (childRef) (childRef as React.RefObject<HTMLElement | null>).current = el;
    },
    onMouseEnter: (e: React.MouseEvent<HTMLElement>) => {
      children.props.onMouseEnter?.(e);
      show(DELAY_MS);
    },
    onMouseLeave: (e: React.MouseEvent<HTMLElement>) => {
      children.props.onMouseLeave?.(e);
      hide();
    },
    onFocus: (e: React.FocusEvent<HTMLElement>) => {
      children.props.onFocus?.(e);
      if (e.currentTarget.matches(":focus-visible")) show(0);
    },
    onBlur: (e: React.FocusEvent<HTMLElement>) => {
      children.props.onBlur?.(e);
      hide();
    },
    onPointerDown: (e: React.PointerEvent<HTMLElement>) => {
      children.props.onPointerDown?.(e);
      hide();
    },
  });

  return (
    <>
      {child}
      {open &&
        createPortal(
          <div
            ref={tip}
            role="tooltip"
            className="ui-tooltip"
            style={{ top: pos?.top ?? -9999, left: pos?.left ?? -9999, visibility: pos ? "visible" : "hidden" }}
          >
            {label}
            {shortcut && <kbd className="ui-kbd">{shortcut}</kbd>}
          </div>,
          document.body,
        )}
    </>
  );
}
