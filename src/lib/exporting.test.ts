import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { conflictText, formatHint, formatLeft, speedFor, timeLeft, withExtension } from "./exporting";

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

  it("names the files already there", () => {
    expect(conflictText(["a-clean.wav"])).toBe(
      "This file already exists in that folder: a-clean.wav. Keep both saves the new one as “name-2”.",
    );
    expect(conflictText(["a", "b", "c", "d"])).toBe(
      "4 of the files already exist in that folder: a, b, c and 1 more. Keep both saves the new ones as “name-2”.",
    );
  });

  it("says audio formats from a video have no picture", () => {
    const video = { is_video: true, mp3: true };
    expect(formatHint("video", video)).toBeUndefined();
    for (const f of ["wav", "flac", "m4a", "mp3"] as const) expect(formatHint(f, video)).toBe("Audio only, no picture");
    expect(formatHint("wav", { is_video: false, mp3: true })).toBeUndefined();
    expect(formatHint("mp3", { is_video: true, mp3: false })).toBe("not available");
  });
});
