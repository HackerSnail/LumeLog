import { describe, expect, it } from "vitest";
import {
  classifyMove,
  extractMacroOps,
  extractSymbols,
  lineAtY,
  MACRO_MAX_OPS,
  MACRO_NAME_MAX,
  MACRO_STORE_MAX,
  overviewBuckets,
  parseMacros,
  parseRepeatCount,
  serializeMacros,
  SYMBOL_PATTERNS,
} from "./editortools";
import type { MacroOp, NamedMacro } from "./editortools";

describe("macro extraction", () => {
  it("folds removed/inserted spans into ordered ops", () => {
    expect(extractMacroOps([{ removed: 3, inserted: "" }])).toEqual([
      { kind: "delete", count: 3, dir: "forward" },
    ]);
    expect(extractMacroOps([{ removed: 0, inserted: "ab" }])).toEqual([{ kind: "insert", text: "ab" }]);
    expect(extractMacroOps([{ removed: 1, inserted: "xy" }])).toEqual([
      { kind: "delete", count: 1, dir: "forward" },
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
      { kind: "delete", count: 2, dir: "forward" },
      { kind: "insert", text: "bc" },
    ]);
  });

  it("records backward deletes for Backspace (Plan 30)", () => {
    expect(extractMacroOps([{ removed: 4, inserted: "" }], "backward")).toEqual([
      { kind: "delete", count: 4, dir: "backward" },
    ]);
  });
});

describe("parseRepeatCount (Plan 31)", () => {
  it("accepts positive integers and clamps", () => {
    expect(parseRepeatCount("3")).toBe(3);
    expect(parseRepeatCount(" 5 ")).toBe(5);
    expect(parseRepeatCount("999999")).toBe(10000);
  });

  it("rejects garbage and non-positive counts", () => {
    expect(parseRepeatCount("abc")).toBeNull();
    expect(parseRepeatCount("")).toBeNull();
    expect(parseRepeatCount("0")).toBeNull();
    expect(parseRepeatCount("-2")).toBeNull();
    expect(parseRepeatCount("2.5")).toBeNull();
  });
});

describe("classifyMove (Plan 30)", () => {
  it("maps same-line ±1 col to left/right", () => {
    expect(classifyMove({ line: 1, col: 5 }, { line: 1, col: 4 }, 10)).toBe("left");
    expect(classifyMove({ line: 1, col: 5 }, { line: 1, col: 6 }, 10)).toBe("right");
  });

  it("maps adjacent-line moves", () => {
    expect(classifyMove({ line: 2, col: 1 }, { line: 1, col: 0 }, 8)).toBe("up");
    expect(classifyMove({ line: 2, col: 1 }, { line: 3, col: 0 }, 8)).toBe("down");
  });

  it("maps column jumps to a boundary as Home/End", () => {
    expect(classifyMove({ line: 1, col: 5 }, { line: 1, col: 0 }, 12)).toBe("line-start");
    expect(classifyMove({ line: 1, col: 5 }, { line: 1, col: 12 }, 12)).toBe("line-end");
  });

  it("returns null for moves it cannot record", () => {
    expect(classifyMove({ line: 1, col: 0 }, { line: 1, col: 0 }, 5)).toBeNull(); // no move
    expect(classifyMove({ line: 1, col: 3 }, { line: 1, col: 7 }, 10)).toBeNull(); // word jump
    expect(classifyMove({ line: 1, col: 5 }, { line: 4, col: 0 }, 5)).toBeNull(); // paging
    expect(classifyMove({ line: 1, col: 0 }, { line: 6, col: 3 }, 5)).toBeNull(); // click
  });
});

describe("macro persistence (Plan 30)", () => {
  it("round-trips through serializeMacros/parseMacros", () => {
    const macros: NamedMacro[] = [
      { name: "加前缀", ops: [{ kind: "insert", text: "X" }] },
      {
        name: "整理",
        ops: [
          { kind: "move", by: "line-start" },
          { kind: "delete", count: 2, dir: "backward" },
          { kind: "line", op: "dedupe" },
        ],
      },
    ];
    expect(parseMacros(serializeMacros(macros))).toEqual(macros);
  });

  it("returns [] for null/corrupt input", () => {
    expect(parseMacros(null)).toEqual([]);
    expect(parseMacros("")).toEqual([]);
    expect(parseMacros("{not json")).toEqual([]);
    expect(parseMacros(JSON.stringify({ macros: 42 }))).toEqual([]);
  });

  it("drops bad macros but keeps good ones", () => {
    const json = JSON.stringify({
      macros: [
        { name: "坏", ops: [{ kind: "teleport" }] },
        { name: "", ops: [{ kind: "insert", text: "a" }] },
        { name: "坏", ops: "nope" },
        { name: "好", ops: [{ kind: "insert", text: "a" }] },
      ],
    });
    expect(parseMacros(json)).toEqual([{ name: "好", ops: [{ kind: "insert", text: "a" }] }]);
  });

  it("enforces store caps", () => {
    const many: NamedMacro[] = Array.from({ length: MACRO_STORE_MAX + 10 }, (_, i) => ({
      name: `宏${i}`,
      ops: [{ kind: "insert", text: "a" }],
    }));
    const parsed = parseMacros(serializeMacros(many));
    expect(parsed).toHaveLength(MACRO_STORE_MAX);

    const longName = "名".repeat(MACRO_NAME_MAX + 10);
    const [one] = parseMacros(serializeMacros([{ name: longName, ops: [] }]));
    expect(one.name).toHaveLength(MACRO_NAME_MAX);

    const bigOps: MacroOp[] = Array.from({ length: MACRO_MAX_OPS + 1 }, () => ({
      kind: "insert",
      text: "a",
    }));
    const [two] = parseMacros(serializeMacros([{ name: "大", ops: bigOps }]));
    expect(two.ops).toHaveLength(MACRO_MAX_OPS);
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
