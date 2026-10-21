import { describe, expect, it } from "vitest";
import { Analysis, formatFloor, formatLufs, formatTruePeak, notes } from "./analysis";

const base: Analysis = {
  integrated_lufs: -23.44,
  loudness_range_lu: 6,
  true_peak_db: -0.32,
  sample_peak_db: -0.5,
  noise_floor_db: -62.2,
  clipped_places: 0,
};

describe("analysis", () => {
  it("formats the readings", () => {
    expect(formatLufs(-23.44)).toBe("−23.4 LUFS");
    expect(formatLufs(null)).toBe("silent");
    expect(formatTruePeak(-0.32)).toBe("−0.3 dBTP");
    expect(formatTruePeak(0.4)).toBe("0.4 dBTP");
    expect(formatTruePeak(-200)).toBe("no peak");
    expect(formatFloor(-62.2)).toBe("−62 dB");
  });

  it("notes only what stands out", () => {
    expect(notes(base)).toEqual([]);
    expect(notes({ ...base, clipped_places: 1 })[0]).toMatch(/^Clipped in 1 place:/);
    expect(notes({ ...base, noise_floor_db: -38, integrated_lufs: -34 })).toEqual([
      "Noticeable background noise.",
      "Recorded quietly.",
    ]);
  });
});
