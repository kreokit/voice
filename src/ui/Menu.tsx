import { useCallback, useEffect, useRef, useState } from "react";
import type { LucideIcon } from "lucide-react";
import { Button, ButtonVariant, ICON_STROKE } from "./Button";
import { nextIndex } from "./logic";
import { Popover } from "./Popover";

export type MenuEntry =
  | {
      label: string;
      icon?: LucideIcon;
      shortcut?: string;
      onSelect: () => void;
      danger?: boolean;
      disabled?: boolean;
    }
  | "separator";

type MenuAction = Exclude<MenuEntry, "separator">;

/** The list inside menus and context menus: arrow keys, Home/End, Enter, Escape. */
function MenuList({ items, onClose, label }: { items: MenuEntry[]; onClose: () => void; label?: string }) {
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const actions = items.filter((i): i is MenuAction => i !== "separator");
  const disabled = (i: number) => !!actions[i]?.disabled;

  useEffect(() => {
    refs.current[nextIndex(-1, 1, actions.length, disabled)]?.focus();
    // Focus the first item once, when the menu opens.
  }, []);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const current = refs.current.findIndex((el) => el === document.activeElement);
    const move = (to: number) => {
      e.preventDefault();
      if (to >= 0) refs.current[to]?.focus();
    };
    switch (e.key) {
      case "ArrowDown":
        return move(nextIndex(current, 1, actions.length, disabled));
      case "ArrowUp":
        return move(nextIndex(current < 0 ? 0 : current, -1, actions.length, disabled));
      case "Home":
        return move(nextIndex(-1, 1, actions.length, disabled));
      case "End":
        return move(nextIndex(0, -1, actions.length, disabled));
      case "Tab":
        e.preventDefault();
        return onClose();
    }
  };

  let a = -1;
  return (
    <div role="menu" aria-label={label} className="ui-menu" onKeyDown={onKeyDown}>
      {items.map((item, i) => {
        if (item === "separator") return <div key={`sep${i}`} role="separator" className="ui-menu-separator" />;
        const index = ++a;
        const Icon = item.icon;
        return (
          <button
            key={item.label}
            ref={(el) => {
              refs.current[index] = el;
            }}
            type="button"
            role="menuitem"
            disabled={item.disabled}
            className={item.danger ? "ui-menu-item ui-menu-item-danger" : "ui-menu-item"}
            onClick={() => {
              onClose();
              item.onSelect();
            }}
          >
            <span className="ui-menu-icon">{Icon && <Icon size={16} strokeWidth={ICON_STROKE} aria-hidden />}</span>
            <span className="ui-menu-label">{item.label}</span>
            {item.shortcut && <kbd className="ui-kbd">{item.shortcut}</kbd>}
          </button>
        );
      })}
    </div>
  );
}

/** A button that opens a menu. */
export function Menu({
  label,
  icon,
  trailingIcon,
  variant = "secondary",
  align = "start",
  items,
  buttonLabel,
  className,
}: {
  /** Visible button text (omit for icon-only; then pass buttonLabel). */
  label?: string;
  /** Accessible name when the button has no text. */
  buttonLabel?: string;
  icon?: LucideIcon;
  trailingIcon?: LucideIcon;
  variant?: ButtonVariant;
  align?: "start" | "end";
  items: MenuEntry[];
  className?: string;
}) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const close = useCallback(() => {
    setOpen(false);
    anchor.current?.focus();
  }, []);

  return (
    <>
      <Button
        ref={anchor}
        variant={variant}
        icon={icon}
        trailingIcon={trailingIcon}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={label ? undefined : buttonLabel}
        className={[label ? undefined : "ui-button-icon-only", className].filter(Boolean).join(" ")}
        onClick={() => setOpen((o) => !o)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setOpen(true);
          }
        }}
      >
        {label}
      </Button>
      <Popover open={open} onClose={close} anchor={anchor} align={align}>
        <MenuList items={items} onClose={close} label={label ?? buttonLabel} />
      </Popover>
    </>
  );
}

/**
 * Right-click menus: spread `onContextMenu` on the target and render `menu`.
 * Items are computed when the menu opens.
 */
export function useContextMenu(getItems: () => MenuEntry[], label?: string) {
  const [state, setState] = useState<{ x: number; y: number; items: MenuEntry[] } | null>(null);
  const restore = useRef<HTMLElement | null>(null);
  const close = useCallback(() => {
    setState(null);
    restore.current?.focus();
  }, []);

  const onContextMenu = (e: React.MouseEvent) => {
    e.preventDefault();
    restore.current = document.activeElement as HTMLElement | null;
    setState({ x: e.clientX, y: e.clientY, items: getItems() });
  };

  const menu = (
    <Popover open={state !== null} onClose={close} point={state ?? undefined}>
      {state && <MenuList items={state.items} onClose={close} label={label} />}
    </Popover>
  );
  return { onContextMenu, menu };
}
