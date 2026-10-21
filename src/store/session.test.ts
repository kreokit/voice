import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { summarize } from "./session";

const info = {
  duration_ms: 1000,
  container: "wav",
  audio: { codec: "pcm_s16le", sample_rate: 48000, channels: 1, channel_layout: "mono" },
  audio_start_ms: 0,
  video: null,
};

describe("summarize", () => {
  it("opens the first usable file and explains the others", () => {
    const result = summarize([
      { path: "/a/silent.mp4", info: null, error: "this file has no sound" },
      { path: "/a/one.wav", info, error: null },
      { path: "/a/two.wav", info, error: null },
      { path: "/a/notes.txt", info: null, error: "this file can't be read as audio or video: …" },
    ]);
    expect(result.open?.path).toBe("/a/one.wav");
    expect(result.others).toBe(1);
    expect(result.messages).toEqual([
      "silent.mp4: it has no sound",
      "notes.txt: this file can't be read as audio or video: …",
    ]);
  });

  it("opens nothing when nothing is usable", () => {
    const result = summarize([{ path: "/a/x.txt", info: null, error: "no" }]);
    expect(result.open).toBeNull();
    expect(result.others).toBe(0);
  });
});
