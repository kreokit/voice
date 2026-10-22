/** Noise removal (src-tauri/src/clean.rs, models.rs) and its settings. */
import { invoke } from "@tauri-apps/api/core";
import { DEFAULT_POLISH, PolishSettings } from "./polish";

export type Model = "fast" | "best";

/** Measured defaults (clean.rs Model::default_strength). */
export const DEFAULT_STRENGTH: Record<Model, number> = { fast: 0.85, best: 0.95 };
/** Best's download size, shown before it is downloaded (models.rs). */
export const BEST_SIZE_MB = 10.5;

export interface CleanMeta {
  version: number;
  model: string;
  frames: number;
  realtime_factor: number;
}

/** What the editor remembers per file (recent.rs FileSettings). */
export interface FileSettings {
  model: Model;
  fast_strength: number;
  best_strength: number;
  polish: PolishSettings;
}

export const DEFAULT_SETTINGS: FileSettings = {
  model: "fast",
  fast_strength: DEFAULT_STRENGTH.fast,
  best_strength: DEFAULT_STRENGTH.best,
  polish: DEFAULT_POLISH,
};

/** The Strength of the file's chosen model. */
export const strengthOf = (s: FileSettings, model: Model = s.model) =>
  model === "fast" ? s.fast_strength : s.best_strength;

/** The settings with `model`'s Strength set. */
export const withStrength = (s: FileSettings, model: Model, strength: number): FileSettings =>
  model === "fast" ? { ...s, fast_strength: strength } : { ...s, best_strength: strength };

export interface CpuSupport {
  supported: boolean;
  missing: string[];
}

export interface BestModel {
  size_bytes: number;
  downloaded: boolean;
  cpu: CpuSupport;
}

export interface DownloadProgress {
  downloaded_bytes: number;
  total_bytes: number;
}

export const bestModel = () => invoke<BestModel>("best_model");
export const downloadBestModel = () => invoke<void>("download_best_model");
export const cancelModelDownload = () => invoke<void>("cancel_model_download");
export const cleanStatus = (path: string, model: Model) => invoke<CleanMeta | null>("clean_status", { path, model });
export const cleanFile = (path: string, model: Model) => invoke<CleanMeta>("clean_file", { path, model });
export const cancelClean = () => invoke<void>("cancel_clean");
export const cleanedWaveform = (path: string, model: Model) =>
  invoke<ArrayBuffer>("cleaned_waveform", { path, model }).then((b) => new Uint8Array(b));
export const fileSettings = (path: string) => invoke<FileSettings>("file_settings", { path });
export const saveSettings = (path: string, settings: FileSettings) =>
  invoke<void>("save_settings", { path, settings });

/** "3.2 of 10.5 MB" */
export const formatDownload = (p: DownloadProgress) =>
  `${(p.downloaded_bytes / 1e6).toFixed(1)} of ${(p.total_bytes / 1e6).toFixed(1)} MB`;

/** Waveform levels are dB-scaled bytes: 0 = −48 dBFS or quieter, 255 = 0 dBFS (preview.rs). */
const FLOOR_DB = -48;
const toAmplitude = (level: number) => (level === 0 ? 0 : 10 ** (((level / 255) * -FLOOR_DB + FLOOR_DB) / 20));
const toLevel = (amplitude: number) =>
  amplitude <= 0 ? 0 : Math.round(Math.min(1, Math.max(0, (20 * Math.log10(amplitude) - FLOOR_DB) / -FLOOR_DB)) * 255);

/**
 * The waveform level of the cleaned sound at `strength` (a blend with the original), from
 * the two peak levels: at most (1 − s)·original + s·cleaned, which is what is drawn.
 */
export function blendLevel(original: number, cleaned: number, strength: number): number {
  return toLevel((1 - strength) * toAmplitude(original) + strength * toAmplitude(cleaned));
}

/** "85%" */
export const formatStrength = (s: number) => `${Math.round(s * 100)}%`;
