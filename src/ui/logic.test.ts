import { describe, expect, it } from "vitest";
import {
  hexToHsv,
  hsvToHex,
  nextIndex,
  normalizeHex,
  parseNumberField,
  placeAtPoint,
  placePopover,
  snapToStep,
  valueAfterKey,
  valueAtRatio,
} from "./logic";

const viewport = { width: 1280, height: 800 };

describe("placePopover", () => {
  const anchor = { top: 100, left: 200, width: 120, height: 32 };

  it("opens below, aligned to the anchor's start or end", () => {
    expect(placePopover(anchor, { width: 240, height: 200 }, viewport)).toEqual({ top: 136, left: 200 });
    expect(placePopover(anchor, { width: 240, height: 200 }, viewport, "end")).toEqual({
      top: 136,
      left: 80,
    });
  });

  it("flips above when there is no room below", () => {
    const low = { ...anchor, top: 700 };
    expect(placePopover(low, { width: 240, height: 200 }, viewport).top).toBe(496);
  });

  it("stays inside the window", () => {
    const right = { ...anchor, left: 1250 };
    expect(placePopover(right, { width: 240, height: 200 }, viewport).left).toBe(1032);
  });
});

describe("placeAtPoint", () => {
  it("opens at the pointer, or up/left near the edges", () => {
    expect(placeAtPoint({ x: 100, y: 100 }, { width: 200, height: 150 }, viewport)).toEqual({
      top: 100,
      left: 100,
    });
    expect(placeAtPoint({ x: 1200, y: 780 }, { width: 200, height: 150 }, viewport)).toEqual({
      top: 630,
      left: 1000,
    });
  });
});

describe("nextIndex", () => {
  it("wraps and skips disabled items", () => {
    expect(nextIndex(0, 1, 3)).toBe(1);
    expect(nextIndex(2, 1, 3)).toBe(0);
    expect(nextIndex(0, -1, 3)).toBe(2);
    expect(nextIndex(0, 1, 4, (i) => i === 1)).toBe(2);
    expect(nextIndex(0, 1, 2, () => true)).toBe(-1);
    expect(nextIndex(0, 1, 0)).toBe(-1);
  });
});

describe("slider values", () => {
  it("snaps to the step grid", () => {
    expect(snapToStep(0.30000000000000004, 0, 1, 0.1)).toBe(0.3);
    expect(snapToStep(57, 24, 160, 4)).toBe(56);
    expect(valueAtRatio(0.5, 24, 160, 1)).toBe(92);
    expect(valueAtRatio(1.4, 0, 12, 1)).toBe(12);
  });

  it("moves by keys", () => {
    expect(valueAfterKey("ArrowRight", false, 5, 0, 12, 1)).toBe(6);
    expect(valueAfterKey("ArrowLeft", true, 5, 0, 12, 1)).toBe(0);
    expect(valueAfterKey("PageUp", false, 40, 24, 160, 1)).toBe(50);
    expect(valueAfterKey("End", false, 40, 24, 160, 1)).toBe(160);
    expect(valueAfterKey("a", false, 40, 24, 160, 1)).toBeNull();
  });
});

describe("normalizeHex", () => {
  it("accepts 3 and 6 digit hex with or without #", () => {
    expect(normalizeHex("#c8f135")).toBe("#C8F135");
    expect(normalizeHex("C8F135")).toBe("#C8F135");
    expect(normalizeHex(" #cf3 ")).toBe("#CCFF33");
  });

  it("rejects everything else", () => {
    for (const bad of ["", "#12", "#12345", "#GGGGGG", "lime", "#1234567"]) {
      expect(normalizeHex(bad), bad).toBeNull();
    }
  });
});

describe("parseNumberField", () => {
  it("reads numbers with an optional % and comma decimals, clamped", () => {
    expect(parseNumberField("12.5", 0, 100)).toBe(12.5);
    expect(parseNumberField(" 12,54 % ", 0, 100)).toBe(12.5);
    expect(parseNumberField("140", 0, 100)).toBe(100);
    expect(parseNumberField("-3", 0, 100)).toBe(0);
    expect(parseNumberField("abc", 0, 100)).toBeNull();
  });
});

describe("HSV colours", () => {
  it("converts known colours", () => {
    expect(hexToHsv("#FF0000")).toEqual({ h: 0, s: 1, v: 1 });
    expect(hexToHsv("#000000")).toEqual({ h: 0, s: 0, v: 0 });
    expect(hexToHsv("#FFFFFF")).toEqual({ h: 0, s: 0, v: 1 });
    const lime = hexToHsv("#C8F135");
    expect(lime.h).toBeCloseTo(73.1, 1);
    expect(hsvToHex({ h: 120, s: 1, v: 1 })).toBe("#00FF00");
    expect(hsvToHex({ h: 240, s: 0.5, v: 0.5 })).toBe("#404080");
  });

  it("round-trips every swatch-like colour", () => {
    for (const hex of ["#C8F135", "#0E0E10", "#3D8BFF", "#FF5FD2", "#A1A1AA", "#123456", "#FEDCBA"]) {
      expect(hsvToHex(hexToHsv(hex))).toBe(hex);
    }
  });
});
