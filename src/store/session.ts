import { create } from "zustand";
import { AddedFile, MediaInfo, addFiles, relocateRecent } from "../lib/media";
import { fileName } from "../lib/files";
import { toast } from "../ui";

/** The open file. */
export interface OpenFile {
  path: string;
  info: MediaInfo;
}

interface Session {
  file: OpenFile | null;
  /** Several files dropped or chosen together: the Batch screen. */
  batch: OpenFile[] | null;
  close: () => void;
}

export const useSession = create<Session>((set) => ({
  file: null,
  batch: null,
  close: () => set({ file: null, batch: null }),
}));

/**
 * What happened to a drop or pick: the file to open (one usable file), the batch (two or
 * more), and messages for the refused ones.
 */
export function summarize(added: AddedFile[]): {
  open: OpenFile | null;
  batch: OpenFile[] | null;
  messages: string[];
  others: number;
} {
  const usable = added.filter((a) => a.info !== null);
  const files = usable.map((a) => ({ path: a.path, info: a.info! }));
  const messages = added
    .filter((a) => a.error !== null)
    .map((a) => `${fileName(a.path)}: ${a.error === "this file has no sound" ? "it has no sound" : a.error}`);
  const first = usable[0];
  return {
    open: first && usable.length === 1 ? { path: first.path, info: first.info! } : null,
    batch: usable.length > 1 ? files : null,
    messages,
    others: Math.max(0, usable.length - 1),
  };
}

/**
 * Adds dropped or chosen files to Recent; opens one usable file in the editor, or two or
 * more on the Batch screen. Returns whether something was opened.
 */
export async function openPaths(paths: string[]): Promise<boolean> {
  if (paths.length === 0) return false;
  let added: AddedFile[];
  try {
    added = await addFiles(paths);
  } catch (e) {
    toast(String(e), "error");
    return false;
  }
  const { open, batch, messages } = summarize(added);
  for (const message of messages.slice(0, 3)) toast(message, "error");
  if (messages.length > 3) toast(`${messages.length - 3} more files couldn't be opened.`, "error");
  if (open) useSession.setState({ file: open, batch: null });
  if (batch) useSession.setState({ file: null, batch });
  return open !== null || batch !== null;
}

/** A moved file found again at `newPath`: it replaces the old entry and opens. */
export async function relocateAndOpen(oldPath: string, newPath: string): Promise<void> {
  try {
    const info = await relocateRecent(oldPath, newPath);
    useSession.setState({ file: { path: newPath, info } });
  } catch (e) {
    toast(String(e), "error");
  }
}
