/** Noise removal (src-tauri/src/clean.rs) and its settings. */
import { invoke } from "@tauri-apps/api/core";

/** Measured default (clean.rs DEFAULT_STRENGTH): noise about 16.5 dB down in the pauses. */
export const DEFAULT_STRENGTH = 0.85;

export interface CleanMeta {
  version: number;
  model: string;
  frames: number;
  realtime_factor: number;
}

export interface FileSettings {
  strength: number;
}

export const cleanStatus = (path: string) => invoke<CleanMeta | null>("clean_status", { path });
export const cleanFile = (path: string) => invoke<CleanMeta>("clean_file", { path });
export const cancelClean = () => invoke<void>("cancel_clean");
export const cleanedWaveform = (path: string) =>
  invoke<ArrayBuffer>("cleaned_waveform", { path }).then((b) => new Uint8Array(b));
export const fileSettings = (path: string) => invoke<FileSettings>("file_settings", { path });
export const saveSettings = (path: string, settings: FileSettings) =>
  invoke<void>("save_settings", { path, settings });

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
