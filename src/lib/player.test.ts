import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { estimate } from "./player";

describe("estimate", () => {
  const state = { playing: true, position_ms: 1_000, duration_ms: 2_000, error: null };

  it("moves on while playing", () => {
    expect(estimate(state, 500, 620)).toBe(1_120);
  });

  it("stays put when paused and never passes the end", () => {
    expect(estimate({ ...state, playing: false }, 500, 900)).toBe(1_000);
    expect(estimate(state, 0, 5_000)).toBe(2_000);
    expect(estimate(state, 500, 400)).toBe(1_000);
  });
});
