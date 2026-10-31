import { describe, expect, it } from "vitest";
import {
  applyLineOp,
  bookmarkNext,
  bookmarkPrev,
  bookmarkToggle,
  clampFontSize,
  type LineOp,
} from "./editorops";

describe("applyLineOp", () => {
  const lines = ["b", "a", "b", "", "c"];

  const cases: Array<[LineOp, string[]]> = [
    ["sort-asc", ["", "a", "b", "b", "c"]],
    ["sort-desc", ["c", "b", "b", "a", ""]],
    ["dedupe", ["b", "a", "", "c"]],
    ["reverse", ["c", "", "b", "a", "b"]],
    ["upper", ["B", "A", "B", "", "C"]],
    ["lower", ["b", "a", "b", "", "c"]],
    ["trim", ["b", "a", "b", "", "c"]],
    ["drop-empty", ["b", "a", "b", "c"]],
    ["join", ["b a b c"]],
  ];

  for (const [op, expected] of cases) {
    it(`applies ${op}`, () => {
      expect(applyLineOp(lines, op)).toEqual(expected);
    });
  }

  it("trims whitespace per line", () => {
    expect(applyLineOp(["  x  ", "\ty"], "trim")).toEqual(["x", "y"]);
  });

  it("does not mutate the input", () => {
    const src = ["b", "a"];
    applyLineOp(src, "sort-asc");
    expect(src).toEqual(["b", "a"]);
  });
});

describe("clampFontSize", () => {
  it("rounds and clamps to [9, 28]", () => {
    expect(clampFontSize(14)).toBe(14);
    expect(clampFontSize(14.4)).toBe(14);
    expect(clampFontSize(3)).toBe(9);
    expect(clampFontSize(50)).toBe(28);
    expect(clampFontSize(NaN)).toBe(14);
  });
});

describe("bookmarks (Plan 28)", () => {
  it("toggles sorted anchors", () => {
    let bms = bookmarkToggle([], 10);
    expect(bms).toEqual([10]);
    bms = bookmarkToggle(bms, 3);
    expect(bms).toEqual([3, 10]); // kept sorted
    bms = bookmarkToggle(bms, 3);
    expect(bms).toEqual([10]); // removed
    bms = bookmarkToggle(bms, 10);
    expect(bms).toEqual([]); // empty again
  });

  it("walks next/prev with wraparound", () => {
    const bms = [10, 40, 70];
    expect(bookmarkNext(bms, 0)).toBe(10);
    expect(bookmarkNext(bms, 10)).toBe(40);
    expect(bookmarkNext(bms, 70)).toBe(10); // wrap
    expect(bookmarkNext(bms, 999)).toBe(10);
    expect(bookmarkPrev(bms, 70)).toBe(40);
    expect(bookmarkPrev(bms, 40)).toBe(10);
    expect(bookmarkPrev(bms, 10)).toBe(70); // wrap
    expect(bookmarkPrev(bms, 0)).toBe(70);
    expect(bookmarkNext([], 5)).toBeNull();
    expect(bookmarkPrev([], 5)).toBeNull();
  });
});
