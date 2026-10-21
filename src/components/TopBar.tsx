import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Bug, Ellipsis, FolderOpen, House } from "lucide-react";
import mark from "../assets/kreokit-mark.png";
import { MEDIA_EXTENSIONS } from "../lib/files";
import { openPaths } from "../store/session";
import { Menu, MenuEntry, toast } from "../ui";

const FILTERS = [{ name: "Audio or video", extensions: MEDIA_EXTENSIONS }];

/** The open-file dialog for one file. Resolves to the picked path, or null. */
export async function pickFile(title = "Open file"): Promise<string | null> {
  const picked = await open({ title, multiple: false, directory: false, filters: FILTERS });
  return typeof picked === "string" ? picked : null;
}

/** Picks one or more files, adds them to Recent and opens the first. */
export async function chooseFiles() {
  const picked = await open({ title: "Open files", multiple: true, directory: false, filters: FILTERS });
  if (picked) await openPaths(Array.isArray(picked) ? picked : [picked]);
}

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
    { label: "Open files…", icon: FolderOpen, shortcut: "⌘O", onSelect: () => void chooseFiles() },
    ...(onHome ? [{ label: "Back to Home", icon: House, onSelect: onHome }] : []),
    "separator",
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
