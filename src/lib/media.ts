/** The backend's view of a file (src-tauri/src/probe.rs, recent.rs). */
import { invoke } from "@tauri-apps/api/core";

export interface AudioInfo {
  codec: string;
  sample_rate: number;
  channels: number;
  channel_layout: string | null;
}

export interface VideoInfo {
  codec: string;
  width: number;
  height: number;
  fps: number;
  rotation: number;
}

export interface MediaInfo {
  duration_ms: number;
  container: string;
  audio: AudioInfo;
  audio_start_ms: number;
  video: VideoInfo | null;
}

export interface RecentEntry {
  path: string;
  opened_at: number;
  duration_ms: number;
  is_video: boolean;
  /** The file still exists. */
  exists: boolean;
}

export interface AddedFile {
  path: string;
  info: MediaInfo | null;
  error: string | null;
}

export const listRecent = () => invoke<RecentEntry[]>("list_recent");
export const addFiles = (paths: string[]) => invoke<AddedFile[]>("add_files", { paths });
/** Removes a file from Recent with KreoKit's data for it (never the file); bytes freed. */
export const removeRecent = (path: string) => invoke<number>("remove_recent", { path });
/** Removes every file from Recent; bytes freed. */
export const clearRecent = () => invoke<number>("clear_recent");

/** "12 KB", "690 MB", "1.2 GB" */
export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${Math.round(bytes / 1e6)} MB`;
  return `${Math.max(0, Math.round(bytes / 1e3))} KB`;
}

/** The toast after removing: "Removed from Recent. 690 MB freed." */
export const removedMessage = (bytes: number, all = false) =>
  `${all ? "Recent files cleared" : "Removed from Recent"}. ${formatBytes(bytes)} freed.`;
export const relocateRecent = (old: string, newPath: string) =>
  invoke<MediaInfo>("relocate_recent", { old, new: newPath });

/** "Stereo · 48 kHz · AAC" */
export function describeAudio(a: AudioInfo): string {
  const channels = a.channels === 1 ? "Mono" : a.channels === 2 ? "Stereo" : `${a.channels} channels`;
  const rate = `${Number((a.sample_rate / 1000).toFixed(1))} kHz`;
  return [channels, rate, a.codec.replace(/^pcm_.*/, "PCM").toUpperCase()].join(" · ");
}
