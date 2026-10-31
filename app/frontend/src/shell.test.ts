import { describe, it, expect } from "vitest";
import { clampWidth, loadFlag, isIMEEvent, MIN_SIDEBAR, MAX_SIDEBAR } from "./shell";

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

describe("isIMEEvent (Plan 39)", () => {
  it("flags isComposing and keyCode 229 separately", () => {
    expect(isIMEEvent({ isComposing: true, keyCode: 13 })).toBe(true);
    // Safari fires the committing Enter after compositionend: isComposing is
    // already false, keyCode 229 is the only signal.
    expect(isIMEEvent({ isComposing: false, keyCode: 229 })).toBe(true);
  });

  it("passes plain keys through", () => {
    expect(isIMEEvent({ isComposing: false, keyCode: 13 })).toBe(false);
    expect(isIMEEvent({ keyCode: 70 })).toBe(false); // plain F, no isComposing
  });
});
