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
  close: () => void;
}

export const useSession = create<Session>((set) => ({
  file: null,
  close: () => set({ file: null }),
}));

/** What happened to a drop or pick: the file to open, and messages for the refused ones. */
export function summarize(added: AddedFile[]): { open: OpenFile | null; messages: string[]; others: number } {
  const usable = added.filter((a) => a.info !== null);
  const messages = added
    .filter((a) => a.error !== null)
    .map((a) => `${fileName(a.path)}: ${a.error === "this file has no sound" ? "it has no sound" : a.error}`);
  const first = usable[0];
  return {
    open: first ? { path: first.path, info: first.info! } : null,
    messages,
    others: Math.max(0, usable.length - 1),
  };
}

/**
 * Adds dropped or chosen files to Recent and opens the first usable one. Returns whether
 * one was opened.
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
  const { open, messages, others } = summarize(added);
  for (const message of messages.slice(0, 3)) toast(message, "error");
  if (messages.length > 3) toast(`${messages.length - 3} more files couldn't be opened.`, "error");
  if (others > 0) toast(`${others} more ${others === 1 ? "file is" : "files are"} in Recent files.`);
  if (open) useSession.setState({ file: open });
  return open !== null;
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
