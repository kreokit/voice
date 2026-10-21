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
export const removeRecent = (path: string) => invoke<void>("remove_recent", { path });
export const relocateRecent = (old: string, newPath: string) =>
  invoke<MediaInfo>("relocate_recent", { old, new: newPath });

/** "Stereo · 48 kHz · AAC" */
export function describeAudio(a: AudioInfo): string {
  const channels = a.channels === 1 ? "Mono" : a.channels === 2 ? "Stereo" : `${a.channels} channels`;
  const rate = `${Number((a.sample_rate / 1000).toFixed(1))} kHz`;
  return [channels, rate, a.codec.replace(/^pcm_.*/, "PCM").toUpperCase()].join(" · ");
}
