import { describe, it, expect } from "vitest";
import { clampWidth, loadFlag, MIN_SIDEBAR, MAX_SIDEBAR } from "./shell";

describe("clampWidth (pane resizer bounds)", () => {
  it("passes an in-range width through", () => {
    expect(clampWidth(300, MIN_SIDEBAR, MAX_SIDEBAR)).toBe(300);
  });

  it("clamps below min and above max", () => {
    expect(clampWidth(10, MIN_SIDEBAR, MAX_SIDEBAR)).toBe(MIN_SIDEBAR);
    expect(clampWidth(9999, MIN_SIDEBAR, MAX_SIDEBAR)).toBe(MAX_SIDEBAR);
  });

  it("treats a non-finite width as min (never NaN into style.width)", () => {
    expect(clampWidth(NaN, 100, 200)).toBe(100);
    expect(clampWidth(Infinity, 100, 200)).toBe(100);
  });

  it("honours min when min === max (locked pane)", () => {
    expect(clampWidth(500, 240, 240)).toBe(240);
  });
});

describe("loadFlag (no storage → caller's default)", () => {
  it("returns the supplied default when storage is unavailable or unset", () => {
    // Node test env has no `localStorage`; the try/catch must fall back.
    expect(loadFlag("lumelog.ui.sidebar", false)).toBe(false);
    expect(loadFlag("lumelog.ui.inspector", true)).toBe(true);
  });
});
