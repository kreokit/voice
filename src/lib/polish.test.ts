import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { DEFAULT_POLISH, afterReadout, samePolish, targetHint } from "./polish";

describe("polish", () => {
  it("reads out the result", () => {
    const m = {
      params: { stages: DEFAULT_POLISH.stages, pre_gain_db: 2, gain_db: 6, limit: true },
      input_lufs: -24,
      output_lufs: -14.04,
      output_true_peak_db: -1.19,
    };
    expect(afterReadout(m)).toEqual({ lufs: "−14.0 LUFS", peak: "peak −1.2 dBTP" });
    expect(afterReadout({ ...m, output_true_peak_db: -3.42 }).peak).toBe("peak −3.4 dBTP");
    expect(afterReadout({ ...m, output_lufs: null }).lufs).toBe("silent");
  });

  it("explains each target and compares settings", () => {
    expect(targetHint(DEFAULT_POLISH)).toMatch(/−14 LUFS/);
    expect(targetHint({ ...DEFAULT_POLISH, target: "off" })).toMatch(/under −1 dBTP/);
    expect(samePolish(DEFAULT_POLISH, { ...DEFAULT_POLISH })).toBe(true);
    expect(samePolish(DEFAULT_POLISH, { ...DEFAULT_POLISH, stages: { ...DEFAULT_POLISH.stages, de_ess: false } })).toBe(false);
  });
});
