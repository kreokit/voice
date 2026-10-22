import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { formatLeft, speedFor, timeLeft, withExtension } from "./exporting";

describe("export", () => {
  it("changes the extension for audio formats only", () => {
    expect(withExtension("/out/talk-clean.wav", "mp3")).toBe("/out/talk-clean.mp3");
    expect(withExtension("/out/clip-clean.mov", "video")).toBe("/out/clip-clean.mov");
  });

  it("estimates the time left per file and in total", () => {
    const speeds = { fast: 0.3, best: 0.7, polish: 0.2 };
    expect(speedFor(speeds, "best")).toBe(0.7);
    expect(speedFor(speeds, null)).toBe(0.2);
    // Three files of 10, 20 and 30 minutes with Best, the second half done.
    const t = timeLeft([600_000, 1_200_000, 1_800_000], 1, 0.5, 0.7);
    expect(t.file).toBeCloseTo(420);
    expect(t.total).toBeCloseTo(420 + 1260);
  });

  it("says the time left plainly", () => {
    expect(formatLeft(4)).toBe("less than 10 s left");
    expect(formatLeft(42)).toBe("about 40 s left");
    expect(formatLeft(420)).toBe("about 7 min left");
    expect(formatLeft(3_600 + 1_260)).toBe("about 1 h 21 min left");
  });
});
