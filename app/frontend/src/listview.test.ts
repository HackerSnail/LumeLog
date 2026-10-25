import { describe, it, expect } from "vitest";
import { visibleWindow, ROW_HEIGHT, OVERSCAN, sliceLineTextByRanges } from "./listview";
import type { Cu16Range } from "./ipc";

const R = (cu16Start: number, cu16Len: number): Cu16Range => ({ cu16Start, cu16Len });

// The virtual list's only headless-testable logic is the scroll→range math;
// DOM rendering is exercised by `tauri dev` in Task 11.
describe("visibleWindow (spec §4.3 read-only list)", () => {
  it("top of file clamps first to 0 and adds the overscan block", () => {
    // 400px viewport = 20 lines visible; ±200 overscan.
    expect(visibleWindow(0, 400, 1000)).toEqual({ first: 0, last: 20 + 1 + OVERSCAN });
  });

  it("mid-scroll returns [line-overscan, line+visible+overscan]", () => {
    const scrollTop = 1000 * ROW_HEIGHT; // scrolled to line 1000
    const w = visibleWindow(scrollTop, 400, 5000);
    expect(w.first).toBe(1000 - OVERSCAN);
    expect(w.last).toBe(1000 + 20 + 1 + OVERSCAN);
  });

  it("clamps to totalLines at the end of the document", () => {
    const scrollTop = 90 * ROW_HEIGHT;
    const w = visibleWindow(scrollTop, 400, 100); // only 100 lines exist
    expect(w.first).toBe(0);
    expect(w.last).toBe(100);
  });

  it("empty document yields an empty window", () => {
    expect(visibleWindow(0, 400, 0)).toEqual({ first: 0, last: 0 });
  });

  it("never inverts: first <= last, both within [0, totalLines]", () => {
    for (const total of [0, 1, 5, 150, 300, 10000]) {
      for (const scrollTop of [0, 37, 4000, total * ROW_HEIGHT]) {
        const { first, last } = visibleWindow(scrollTop, 400, total);
        expect(first).toBeLessThanOrEqual(last);
        expect(first).toBeGreaterThanOrEqual(0);
        expect(last).toBeLessThanOrEqual(total);
      }
    }
  });
});

describe("sliceLineTextByRanges (M3 spec §4.2, pure)", () => {
  it("splits a line into highlighted + plain segments", () => {
    expect(sliceLineTextByRanges("ERROR x", [R(0, 5)])).toEqual([
      { text: "ERROR", hl: true },
      { text: " x", hl: false },
    ]);
  });

  it("merges overlapping ranges (no character emitted twice)", () => {
    const segs = sliceLineTextByRanges("abcdef", [R(1, 3), R(2, 3)]);
    expect(segs).toEqual([
      { text: "a", hl: false },
      { text: "bcde", hl: true },
      { text: "f", hl: false },
    ]);
  });

  it("drops out-of-range / empty spans", () => {
    expect(sliceLineTextByRanges("abc", [R(2, 99)])).toEqual([
      { text: "ab", hl: false },
      { text: "c", hl: true },
    ]);
    expect(sliceLineTextByRanges("abc", [R(1, 0)])).toEqual([{ text: "abc", hl: false }]);
  });

  it("empty ranges yield the whole line unstyled", () => {
    expect(sliceLineTextByRanges("hi", [])).toEqual([{ text: "hi", hl: false }]);
  });

  it("astral char (surrogate pair) sliced at char-aligned cu16 stays intact", () => {
    // "a😀b": a=1 cu, 😀=2 cu (indices 1..3), b=1 cu (index 3).
    const line = "a\u{1F600}b";
    const segs = sliceLineTextByRanges(line, [R(1, 2)]);
    expect(segs).toEqual([
      { text: "a", hl: false },
      { text: "\u{1F600}", hl: true },
      { text: "b", hl: false },
    ]);
  });
});
