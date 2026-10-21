import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { DEFAULT_SETTINGS, blendLevel, formatDownload, formatStrength, strengthOf, withStrength } from "./clean";

describe("blendLevel", () => {
  it("is the original at 0 and the cleaned sound at 1", () => {
    expect(blendLevel(200, 50, 0)).toBe(200);
    expect(blendLevel(200, 50, 1)).toBe(50);
    expect(blendLevel(0, 0, 0.5)).toBe(0);
  });

  it("leaves the noise 20·log10(1 − s) dB down where the cleaned sound is silent", () => {
    // Original at −24 dBFS (level 127.5), cleaned silent, 85%: −24 − 16.5 = −40.5 dBFS.
    const level = blendLevel(128, 0, 0.85);
    expect((level / 255) * 48 - 48).toBeCloseTo(-40.5, 0);
  });

  it("formats the strength", () => {
    expect(formatStrength(0.85)).toBe("85%");
    expect(formatStrength(1)).toBe("100%");
  });
});

describe("settings", () => {
  it("keeps a Strength per model", () => {
    expect(strengthOf(DEFAULT_SETTINGS)).toBe(0.85);
    expect(strengthOf({ ...DEFAULT_SETTINGS, model: "best" })).toBe(0.95);
    const s = withStrength(DEFAULT_SETTINGS, "best", 0.7);
    expect([s.fast_strength, s.best_strength]).toEqual([0.85, 0.7]);
  });

  it("shows download progress", () => {
    expect(formatDownload({ downloaded_bytes: 3_200_000, total_bytes: 10_493_337 })).toBe("3.2 of 10.5 MB");
  });
});
