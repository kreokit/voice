/** Polish settings and the loudness measurement (src-tauri/src/polish.rs, loudness.rs). */
import { invoke } from "@tauri-apps/api/core";
import type { Model } from "./clean";

export type Target = "social" | "podcast" | "custom" | "off";

export interface Stages {
  rumble: boolean;
  de_ess: boolean;
  compress: boolean;
}

export interface PolishSettings {
  stages: Stages;
  target: Target;
  custom_lufs: number;
}

export const DEFAULT_POLISH: PolishSettings = {
  stages: { rumble: true, de_ess: true, compress: true },
  target: "social",
  custom_lufs: -14,
};

export interface Measured {
  params: { stages: Stages; pre_gain_db: number; gain_db: number; limit: boolean };
  input_lufs: number | null;
  output_lufs: number | null;
  output_true_peak_db: number;
}

export const TARGETS: { value: Target; label: string; hint: string }[] = [
  { value: "social", label: "Social video", hint: "−14 LUFS" },
  { value: "podcast", label: "Podcast", hint: "−16 LUFS" },
  { value: "custom", label: "Custom", hint: "" },
  { value: "off", label: "Keep the original level", hint: "" },
];

/** What the target means, for the hint under the choice. */
export function targetHint(s: PolishSettings): string {
  switch (s.target) {
    case "social":
      return "TikTok, Reels, Shorts and YouTube play voices at about −14 LUFS.";
    case "podcast":
      return "Apple Podcasts and Spotify recommend about −16 LUFS.";
    case "custom":
      return "Between −30 and −9 LUFS.";
    case "off":
      return "The level stays as recorded; peaks are still kept under −1 dBTP.";
  }
}

/** "After: −14.0 LUFS · peak −1.2 dBTP" readout parts (as measured). */
export function afterReadout(m: Measured): { lufs: string; peak: string } {
  const minus = (s: string) => s.replace(/^-/, "−");
  return {
    lufs: m.output_lufs === null ? "silent" : `${minus(m.output_lufs.toFixed(1))} LUFS`,
    peak: `peak ${minus(m.output_true_peak_db.toFixed(1))} dBTP`,
  };
}

export const measurePolish = (path: string, cleaned: Model | null, strength: number, settings: PolishSettings) =>
  invoke<Measured>("measure_polish", { path, cleaned, strength, settings });
export const cancelPolish = () => invoke<void>("cancel_polish");

/** Same settings, so no new measurement is needed. */
export const samePolish = (a: PolishSettings, b: PolishSettings) =>
  a.target === b.target &&
  a.custom_lufs === b.custom_lufs &&
  a.stages.rumble === b.stages.rumble &&
  a.stages.de_ess === b.stages.de_ess &&
  a.stages.compress === b.stages.compress;
