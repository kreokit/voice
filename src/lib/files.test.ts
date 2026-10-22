import { describe, expect, it } from "vitest";
import { fileName, formatDuration, formatTime, isMediaPath, relativeTime } from "./files";
import { describeAudio, formatBytes, removedMessage } from "./media";

describe("files", () => {
  it("accepts audio and video by extension, in any case", () => {
    expect(isMediaPath("/a/podcast.WAV")).toBe(true);
    expect(isMediaPath("C:\\rec\\take.m4a")).toBe(true);
    expect(isMediaPath("/a/clip.mov")).toBe(true);
    expect(isMediaPath("/a/notes.txt")).toBe(false);
    expect(isMediaPath("/a/noextension")).toBe(false);
  });

  it("names files from either kind of path", () => {
    expect(fileName("/Users/me/Episode 12.wav")).toBe("Episode 12.wav");
    expect(fileName("C:\\Users\\me\\take.mp4")).toBe("take.mp4");
  });

  it("formats lengths and playhead times", () => {
    expect(formatDuration(4_400)).toBe("0:04");
    expect(formatDuration(4_600)).toBe("0:05");
    expect(formatDuration(725_000)).toBe("12:05");
    expect(formatDuration(3_723_000)).toBe("1:02:03");
    expect(formatTime(4_290)).toBe("0:04.2");
    expect(formatTime(3_723_550)).toBe("1:02:03.5");
    expect(formatTime(-5)).toBe("0:00.0");
  });

  it("says when a file was opened", () => {
    const now = new Date(2026, 9, 21, 15, 0).getTime();
    expect(relativeTime(now - 30_000, now)).toBe("just now");
    expect(relativeTime(now - 5 * 60_000, now)).toBe("5 min ago");
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe("3 h ago");
    expect(relativeTime(new Date(2026, 9, 20, 9, 0).getTime(), now)).toBe("yesterday");
    expect(relativeTime(new Date(2026, 9, 2).getTime(), now)).toBe("2 Oct");
  });
});

describe("media", () => {
  it("describes the sound", () => {
    expect(describeAudio({ codec: "aac", sample_rate: 48_000, channels: 2, channel_layout: "stereo" })).toBe(
      "Stereo · 48 kHz · AAC",
    );
    expect(describeAudio({ codec: "pcm_s24le", sample_rate: 44_100, channels: 1, channel_layout: null })).toBe(
      "Mono · 44.1 kHz · PCM",
    );
    expect(describeAudio({ codec: "flac", sample_rate: 96_000, channels: 6, channel_layout: "5.1" })).toBe(
      "6 channels · 96 kHz · FLAC",
    );
  });
});

describe("freed space", () => {
  it("formats bytes and the message", () => {
    expect(formatBytes(12_345)).toBe("12 KB");
    expect(formatBytes(691_560_000)).toBe("692 MB");
    expect(formatBytes(1_234_000_000)).toBe("1.2 GB");
    expect(formatBytes(0)).toBe("0 KB");
    expect(removedMessage(691_560_000)).toBe("Removed from Recent. 692 MB freed.");
    expect(removedMessage(1_234_000_000, true)).toBe("Recent files cleared. 1.2 GB freed.");
  });
});
