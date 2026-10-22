/** Export (src-tauri/src/export.rs) and Export all: types, commands, time left. */
import { invoke } from "@tauri-apps/api/core";
import type { Model } from "./clean";
import type { PolishSettings } from "./polish";

export type Format = "wav" | "flac" | "m4a" | "mp3" | "video";
export type Stage = "cleaning" | "measuring" | "writing" | "checking";

export interface Done {
  path: string;
  lufs: number | null;
  true_peak_db: number;
  duration_ms: number;
  bytes: number;
  realtime_factor: number;
}

export interface ExportOptions {
  format: Format;
  destination: string;
  exists: boolean;
  nothing_to_do: boolean;
  cleaned: Model | null;
  mp3: boolean;
  is_video: boolean;
}

export interface ExportSettings {
  cleaned: Model | null;
  strength: number;
  polish: PolishSettings;
}

export interface Speeds {
  fast: number;
  best: number;
  polish: number;
}

export interface PlanItem {
  path: string;
  destination: string;
  format: Format;
  exists: boolean;
  duration_ms: number;
  bytes: number;
  error: string | null;
}

export interface Plan {
  items: PlanItem[];
  nothing_to_do: boolean;
  total_bytes: number;
  free_bytes: number | null;
  margin_bytes: number;
  speeds: Speeds;
}

export interface BatchItem {
  path: string;
  destination: string;
  format: Format;
  replace: boolean;
}

export interface BatchResult {
  path: string;
  done: Done | null;
  error: string | null;
}

export interface ExportProgress {
  path: string;
  index: number;
  stage: Stage;
  fraction: number;
}

export const CANCELLED = "cancelled";
export const NOTHING_TO_DO = "nothing would change";

export const exportOptions = (path: string) => invoke<ExportOptions>("export_options", { path });
export const exportFile = (path: string, format: Format, destination: string, replace: boolean) =>
  invoke<Done>("export_file", { path, format, destination, replace });
export const cancelExport = () => invoke<void>("cancel_export");
export const exportPlan = (paths: string[], folder: string, format: Format | null, settings: ExportSettings) =>
  invoke<Plan>("export_plan", { paths, folder, format, settings });
export const exportBatch = (items: BatchItem[], settings: ExportSettings) =>
  invoke<BatchResult[]>("export_batch", { items, settings });
export const revealExport = (path: string) => invoke<void>("reveal_export", { path });
export const devExportFolder = () => invoke<string | null>("dev_export_folder");
export const freeName = (path: string) => invoke<string>("free_name", { path });

export const FORMAT_LABELS: Record<Format, string> = {
  wav: "WAV (24-bit)",
  flac: "FLAC (24-bit)",
  m4a: "M4A (AAC 256 kbps)",
  mp3: "MP3 (320 kbps)",
  video: "Video (picture kept as it is)",
};

export const STAGE_LABELS: Record<Stage, string> = {
  cleaning: "Removing noise",
  measuring: "Measuring the loudness",
  writing: "Writing the file",
  checking: "Checking the result",
};

/** The file's extension for a format ("mp4" etc. for videos come from the file). */
export function withExtension(path: string, format: Format): string {
  if (format === "video") return path;
  return path.replace(/\.[^./\\]+$/, "") + "." + format;
}

/** Seconds of work per second of sound for these settings (learned speeds). */
export const speedFor = (speeds: Speeds, cleaned: Model | null) =>
  cleaned === "fast" ? speeds.fast : cleaned === "best" ? speeds.best : speeds.polish;

/**
 * Time left (s): the current file's remaining share plus the files after it, from the
 * measured speed. `durationsMs` are all files' lengths, `index` the current one.
 */
export function timeLeft(durationsMs: number[], index: number, fraction: number, speed: number): {
  file: number;
  total: number;
} {
  const work = (ms: number) => (ms / 1000) * speed;
  const file = work(durationsMs[index] ?? 0) * (1 - Math.min(Math.max(fraction, 0), 1));
  const rest = durationsMs.slice(index + 1).reduce((sum, ms) => sum + work(ms), 0);
  return { file, total: file + rest };
}

/** "about 3 min left", "about 40 s left", "less than 10 s left". */
export function formatLeft(seconds: number): string {
  if (seconds < 10) return "less than 10 s left";
  if (seconds < 90) return `about ${Math.round(seconds / 5) * 5} s left`;
  if (seconds < 3600) return `about ${Math.round(seconds / 60)} min left`;
  const h = Math.floor(seconds / 3600);
  const m = Math.round((seconds % 3600) / 60);
  return `about ${h} h ${m} min left`;
}

/** The batch conflict message for the names that already exist (at least one). */
export function conflictText(names: string[]): string {
  const listed = names.slice(0, 3).join(", ") + (names.length > 3 ? ` and ${names.length - 3} more` : "");
  const which = names.length === 1 ? "This file already exists" : `${names.length} of the files already exist`;
  const keep = names.length === 1 ? "the new one" : "the new ones";
  return `${which} in that folder: ${listed}. Keep both saves ${keep} as “name-2”.`;
}

/** The note next to a format: audio formats from a video drop the picture. */
export function formatHint(format: Format, options: { is_video: boolean; mp3: boolean } | null): string | undefined {
  if (format === "mp3" && options && !options.mp3) return "not available";
  if (format !== "video" && options?.is_video) return "Audio only, no picture";
  return undefined;
}
