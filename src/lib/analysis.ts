/** A recording's measurements (src-tauri/src/analysis.rs). */

export interface Analysis {
  integrated_lufs: number | null;
  loudness_range_lu: number | null;
  true_peak_db: number;
  sample_peak_db: number;
  noise_floor_db: number;
  clipped_places: number;
}

/** analysis.rs SILENT_DB */
const SILENT_DB = -200;

/** "−23.4 LUFS" (a real minus sign); "silent" without loudness. */
export const formatLufs = (lufs: number | null) => (lufs === null ? "silent" : `${minus(lufs.toFixed(1))} LUFS`);

/** "−0.3 dBTP" */
export const formatTruePeak = (db: number) => (db <= SILENT_DB ? "no peak" : `${minus(db.toFixed(1))} dBTP`);

/** "−62 dB" */
export const formatFloor = (db: number) => (db <= SILENT_DB ? "silent" : `${minus(db.toFixed(0))} dB`);

const minus = (s: string) => s.replace(/^-/, "−");

/**
 * Plain-language notes on the original sound: clipping, a high noise floor, a very quiet
 * or loud level. Empty when nothing stands out.
 */
export function notes(a: Analysis): string[] {
  const out: string[] = [];
  if (a.clipped_places > 0)
    out.push(
      `Clipped in ${a.clipped_places} ${a.clipped_places === 1 ? "place" : "places"}: the recording was too loud there, and that distortion can't be fully undone.`,
    );
  if (a.noise_floor_db > -45 && a.noise_floor_db > SILENT_DB) out.push("Noticeable background noise.");
  if (a.integrated_lufs !== null && a.integrated_lufs < -30) out.push("Recorded quietly.");
  return out;
}
