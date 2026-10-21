import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { placeAtPoint, placePopover } from "./logic";

interface PopoverProps {
  open: boolean;
  onClose: () => void;
  /** Positioned under this element… */
  anchor?: React.RefObject<HTMLElement | null>;
  /** …or at this point (context menus). */
  point?: { x: number; y: number };
  align?: "start" | "end";
  /** Match the anchor's width (selects). */
  matchWidth?: boolean;
  className?: string;
  children: React.ReactNode;
}

/**
 * A floating panel in a portal, placed under its anchor (flipping above when needed) or
 * at a point. Closes on Escape and on pointer-down outside it and its anchor.
 */
export function Popover({ open, onClose, anchor, point, align, matchWidth, className, children }: PopoverProps) {
  const panel = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);

  useLayoutEffect(() => {
    if (!open || !panel.current) return;
    const size = { width: panel.current.offsetWidth, height: panel.current.offsetHeight };
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    if (point) {
      setPosition(placeAtPoint(point, size, viewport));
    } else if (anchor?.current) {
      const r = anchor.current.getBoundingClientRect();
      setPosition(placePopover({ top: r.top, left: r.left, width: r.width, height: r.height }, size, viewport, align));
    }
  }, [open, point, anchor, align]);

  useEffect(() => {
    if (!open) return;
    const onPointer = (e: PointerEvent) => {
      const target = e.target as Node;
      if (panel.current?.contains(target) || anchor?.current?.contains(target)) return;
      onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    const onBlur = () => onClose();
    document.addEventListener("pointerdown", onPointer, true);
    document.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onBlur);
    return () => {
      document.removeEventListener("pointerdown", onPointer, true);
      document.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onBlur);
    };
  }, [open, onClose, anchor]);

  useEffect(() => {
    if (!open) setPosition(null);
  }, [open]);

  if (!open) return null;
  const width = matchWidth && anchor?.current ? anchor.current.offsetWidth : undefined;
  return createPortal(
    <div
      ref={panel}
      className={["ui-popover", className].filter(Boolean).join(" ")}
      style={{
        top: position?.top ?? -9999,
        left: position?.left ?? -9999,
        minWidth: width,
        visibility: position ? "visible" : "hidden",
      }}
    >
      {children}
    </div>,
    document.body,
  );
}
