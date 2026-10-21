import { describe, expect, it } from "vitest";
import { clampView, fitView, follow, msAt, tickLabel, tickStep, xAt, zoomAt } from "./timeline";

describe("view", () => {
  it("converts between time and pixels", () => {
    const view = { startMs: 1000, msPerPx: 10 };
    expect(xAt(1500, view)).toBe(50);
    expect(msAt(50, view)).toBe(1500);
  });

  it("fits the video and stays inside it", () => {
    expect(fitView(10_000, 1000)).toEqual({ startMs: 0, msPerPx: 10 });
    expect(clampView({ startMs: -500, msPerPx: 5 }, 10_000, 1000)).toEqual({ startMs: 0, msPerPx: 5 });
    expect(clampView({ startMs: 9000, msPerPx: 5 }, 10_000, 1000)).toEqual({ startMs: 5000, msPerPx: 5 });
    expect(clampView({ startMs: 0, msPerPx: 50 }, 10_000, 1000).msPerPx).toBe(10);
    expect(clampView({ startMs: 0, msPerPx: 0.5 }, 10_000, 1000).msPerPx).toBe(2);
  });

  it("zooms around the pointer", () => {
    const view = zoomAt({ startMs: 0, msPerPx: 10 }, 2, 400, 10_000, 1000);
    expect(view.msPerPx).toBe(5);
    expect(msAt(400, view)).toBe(4000); // the time under the pointer stays put
  });

  it("pages to follow the playhead", () => {
    const view = { startMs: 0, msPerPx: 5 }; // shows 0–5000 in 1000 px
    expect(follow(view, 4000, 10_000, 1000)).toBe(view);
    expect(follow(view, 4600, 10_000, 1000).startMs).toBe(4100);
  });

  it("picks readable ruler steps and labels", () => {
    expect(tickStep(10)).toBe(1000); // 100 px per second
    expect(tickStep(100)).toBe(10_000);
    expect(tickStep(1)).toBe(100);
    expect(tickLabel(65_000, 5000)).toBe("1:05");
    expect(tickLabel(1500, 500)).toBe("0:01.5");
    expect(tickLabel(3_723_000, 60_000)).toBe("1:02:03");
  });
});
