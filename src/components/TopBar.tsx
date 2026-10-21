import { invoke } from "@tauri-apps/api/core";
import { Bug, Ellipsis } from "lucide-react";
import mark from "../assets/kreokit-mark.png";
import { Menu, MenuEntry, toast } from "../ui";

/**
 * The 48 px app bar: logo mark, title, the ⋯ menu and screen actions (`children`).
 * Empty parts are window drag regions; macOS draws the window buttons inside it.
 */
export function TopBar({
  title,
  onHome,
  children,
}: {
  title: React.ReactNode;
  /** Adds "Back to Home" to the menu. */
  onHome?: () => void;
  children?: React.ReactNode;
}) {
  const items: MenuEntry[] = [
    ...(onHome ? [{ label: "Back to Home", onSelect: onHome }, "separator" as const] : []),
    {
      label: "Report a problem",
      icon: Bug,
      onSelect: () => void invoke("report_problem").catch((e) => toast(String(e), "error")),
    },
  ];
  return (
    <header className="topbar" data-tauri-drag-region>
      <img src={mark} alt="" className="topbar-mark" width={24} height={24} data-tauri-drag-region />
      <div className="topbar-title" data-tauri-drag-region>
        {title}
      </div>
      <div className="topbar-actions">
        <Menu buttonLabel="App menu" icon={Ellipsis} variant="ghost" align="end" items={items} />
        {children}
      </div>
    </header>
  );
}
