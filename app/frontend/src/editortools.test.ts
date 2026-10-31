import { describe, expect, it } from "vitest";
import {
  extractMacroOps,
  extractSymbols,
  lineAtY,
  overviewBuckets,
  SYMBOL_PATTERNS,
} from "./editortools";

describe("macro extraction", () => {
  it("folds removed/inserted spans into ordered ops", () => {
    expect(extractMacroOps([{ removed: 3, inserted: "" }])).toEqual([{ kind: "delete", count: 3 }]);
    expect(extractMacroOps([{ removed: 0, inserted: "ab" }])).toEqual([{ kind: "insert", text: "ab" }]);
    expect(extractMacroOps([{ removed: 1, inserted: "xy" }])).toEqual([
      { kind: "delete", count: 1 },
      { kind: "insert", text: "xy" },
    ]);
    expect(extractMacroOps([])).toEqual([]);
  });

  it("keeps multi-range transactions in order", () => {
    expect(
      extractMacroOps([
        { removed: 0, inserted: "a" },
        { removed: 2, inserted: "" },
        { removed: 0, inserted: "bc" },
      ]),
    ).toEqual([
      { kind: "insert", text: "a" },
      { kind: "delete", count: 2 },
      { kind: "insert", text: "bc" },
    ]);
  });
});

describe("overview buckets", () => {
  it("maps one row per line when lines fit", () => {
    const b = overviewBuckets(5, 100);
    expect(b).toHaveLength(5);
    expect(b[0]).toEqual([1, 1]);
    expect(b[4]).toEqual([5, 5]);
  });

  it("compresses many lines into rows", () => {
    const b = overviewBuckets(1000, 100);
    expect(b).toHaveLength(100);
    expect(b[0]).toEqual([1, 10]);
    expect(b[99]).toEqual([991, 1000]);
  });

  it("click y → line and degenerate inputs", () => {
    expect(lineAtY(0, 100, 1000)).toBe(1);
    expect(lineAtY(50, 100, 1000)).toBe(501);
    expect(lineAtY(100, 100, 1000)).toBe(1000);
    expect(lineAtY(-5, 100, 1000)).toBe(1);
    expect(overviewBuckets(0, 100)).toEqual([[1, 1]]);
    expect(lineAtY(10, 100, 0)).toBe(1);
  });
});

describe("symbol extraction", () => {
  const doc = [
    { n: 1, text: "import os" },
    { n: 2, text: "def start_pipeline(arg):" },
    { n: 3, text: "class Runner:" },
    { n: 4, text: "    x = 1" },
  ];

  it("extracts python defs and classes", () => {
    expect(extractSymbols(doc, "python")).toEqual([
      { name: "start_pipeline", line: 2, kind: "function" },
      { name: "Runner", line: 3, kind: "class" },
    ]);
  });

  it("extracts rust fns and markdown headings", () => {
    const rust = [
      { n: 1, text: "pub async fn run_all() {" },
      { n: 2, text: "pub struct Sensor;" },
    ];
    expect(extractSymbols(rust, "rust")).toEqual([
      { name: "run_all", line: 1, kind: "function" },
      { name: "Sensor", line: 2, kind: "class" },
    ]);
    const md = [
      { n: 1, text: "# 标题" },
      { n: 2, text: "正文" },
    ];
    expect(extractSymbols(md, "markdown")).toEqual([
      { name: "标题", line: 1, kind: "heading" },
    ]);
  });

  it("yields nothing for unknown or plain modes", () => {
    expect(extractSymbols(doc, "plain")).toEqual([]);
    expect(extractSymbols(doc, null)).toEqual([]);
    expect(SYMBOL_PATTERNS["log"]).toBeUndefined();
  });
});
