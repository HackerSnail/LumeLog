// Pure helpers for the editor tools (Plan 29 recording/overview/symbols; Plan
// 30 macro actions + persistence). DOM-free so every rule is unit-testable;
// the CodeMirror glue lives in editor.ts / main.ts.

import type { LineOp } from "./editorops";

// ---- macros ------------------------------------------------------------------

/** Named cursor moves recordable in a macro (Plan 30 words + Plan 32 select). */
export type MoveBy =
  | "left"
  | "right"
  | "up"
  | "down"
  | "line-start"
  | "line-end"
  | "word-left"
  | "word-right";

export type MacroOp =
  | { kind: "insert"; text: string }
  | { kind: "delete"; count: number; dir: "forward" | "backward" }
  | { kind: "del-word"; dir: "forward" | "backward" }
  | { kind: "move"; by: MoveBy; /** true when recorded as a selection extension */ select?: boolean }
  | { kind: "line"; op: LineOp };

/** Hard cap so a runaway recording cannot exhaust memory. */
export const MACRO_MAX_OPS = 10_000;

/**
 * Fold one transaction's changed ranges into macro ops, in document order.
 * Ranges come from `tr.changes.iterChangedRanges` — `removed` is the deleted
 * span length, `inserted` the replacing text.
 */
export function extractMacroOps(
  ranges: ReadonlyArray<{ removed: number; inserted: string }>,
  dir: "forward" | "backward" = "forward",
): MacroOp[] {
  const ops: MacroOp[] = [];
  for (const r of ranges) {
    if (r.removed > 0) ops.push({ kind: "delete", count: r.removed, dir });
    if (r.inserted.length > 0) ops.push({ kind: "insert", text: r.inserted });
  }
  return ops;
}

/** Head position in line/column terms, fed to `classifyMove`. */
export interface MoveProbe {
  /** 1-based line number. */
  line: number;
  /** Column within the line (0 = line start). */
  col: number;
}

/**
 * Classify one cursor move from before/after probes. Only single-step moves
 * are recordable: same-line ±1 col, adjacent line, or Home/End (column jump
 * to a line boundary). Word jumps, paging, and wrap-boundary moves yield null
 * (known limitation, spec §1).
 */
export function classifyMove(
  before: MoveProbe,
  after: MoveProbe,
  afterLineLen: number,
): MoveBy | null {
  if (after.line === before.line) {
    if (Math.abs(after.col - before.col) === 1) return after.col > before.col ? "right" : "left";
    if (before.col !== 0 && after.col === 0) return "line-start";
    if (afterLineLen > 0 && after.col === afterLineLen && before.col < afterLineLen) {
      return "line-end";
    }
    return null;
  }
  if (after.line === before.line - 1) return "up";
  if (after.line === before.line + 1) return "down";
  return null;
}

/** Playback guard: one command may replay a macro at most this many times. */
export const MACRO_PLAY_MAX = 10_000;

/** "12" → 12 (clamped to MACRO_PLAY_MAX); anything else → null. */
export function parseRepeatCount(raw: string): number | null {
  const s = raw.trim();
  if (!/^\d+$/.test(s)) return null;
  const n = Number.parseInt(s, 10);
  if (n < 1) return null;
  return Math.min(n, MACRO_PLAY_MAX);
}

// ---- macro persistence (Plan 30) ----------------------------------------------

export interface NamedMacro {
  name: string;
  ops: MacroOp[];
}

/** Storage guard: at most this many macros per store. */
export const MACRO_STORE_MAX = 50;
/** Storage guard: names are truncated to this length. */
export const MACRO_NAME_MAX = 100;

const MOVE_BYS: readonly MoveBy[] = [
  "left",
  "right",
  "up",
  "down",
  "line-start",
  "line-end",
  "word-left",
  "word-right",
];
const LINE_OPS: readonly LineOp[] = [
  "sort-asc",
  "sort-desc",
  "dedupe",
  "reverse",
  "upper",
  "lower",
  "trim",
  "drop-empty",
  "join",
];

function isMacroOp(v: unknown): v is MacroOp {
  if (!v || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  if (o.kind === "insert") return typeof o.text === "string";
  if (o.kind === "delete") {
    return (
      typeof o.count === "number" &&
      o.count > 0 &&
      (o.dir === "forward" || o.dir === "backward")
    );
  }
  if (o.kind === "del-word") return o.dir === "forward" || o.dir === "backward";
  if (o.kind === "move") {
    return MOVE_BYS.includes(o.by as MoveBy) && (o.select === undefined || typeof o.select === "boolean");
  }
  if (o.kind === "line") return LINE_OPS.includes(o.op as LineOp);
  return false;
}

export function serializeMacros(macros: readonly NamedMacro[]): string {
  return JSON.stringify({ v: 1, macros: macros.map((m) => ({ name: m.name, ops: m.ops })) });
}

/**
 * Tolerant parse: one bad macro is dropped, not fatal; caps enforced so a
 * hand-edited or oversized store cannot exhaust memory. Corrupt JSON → [].
 */
export function parseMacros(json: string | null): NamedMacro[] {
  if (!json) return [];
  let raw: { macros?: unknown };
  try {
    raw = JSON.parse(json) as { macros?: unknown };
  } catch {
    return [];
  }
  if (!raw || typeof raw !== "object" || !Array.isArray(raw.macros)) return [];
  const out: NamedMacro[] = [];
  for (const m of raw.macros) {
    if (out.length >= MACRO_STORE_MAX) break;
    if (!m || typeof m !== "object") continue;
    const name = (m as { name?: unknown }).name;
    const ops = (m as { ops?: unknown }).ops;
    if (typeof name !== "string" || name.trim() === "" || !Array.isArray(ops)) continue;
    if (!ops.every(isMacroOp)) continue;
    out.push({
      name: name.slice(0, MACRO_NAME_MAX),
      ops: (ops as MacroOp[]).slice(0, MACRO_MAX_OPS),
    });
  }
  return out;
}

// ---- overview strip (document map) -------------------------------------------

/**
 * Line ranges per overview pixel row: line count compressed into `px` rows.
 * With more rows than lines, several rows map to the same single line.
 */
export function overviewBuckets(
  totalLines: number,
  px: number,
): Array<[number, number]> {
  const rows = Math.max(1, Math.min(Math.max(1, px), Math.max(1, totalLines)));
  const per = totalLines / rows;
  return Array.from({ length: rows }, (_, i) => {
    const start = Math.floor(i * per) + 1;
    const end = Math.min(totalLines, Math.max(1, Math.floor((i + 1) * per)));
    return [start, Math.max(start, end)] as [number, number];
  });
}

/** Inverse mapping: overview y (CSS px) → 1-based line. */
export function lineAtY(y: number, px: number, totalLines: number): number {
  if (totalLines <= 0) return 1;
  return Math.min(totalLines, Math.max(1, Math.floor((y / Math.max(1, px)) * totalLines) + 1));
}

/** Performance guard: above this line count the strip draws only the viewport. */
export const OVERVIEW_MAX_LINES = 500_000;

// ---- symbol extraction (function list) ---------------------------------------

export interface SymbolPattern {
  /** Matched against each line; group 1 (or the whole match) is the symbol. */
  re: RegExp;
  kind: "function" | "class" | "heading";
}

/** Per builtin mode id; modes absent here have no symbol rules. */
export const SYMBOL_PATTERNS: Record<string, SymbolPattern[]> = {
  javascript: [
    { re: /^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*(\w+)/, kind: "function" },
    { re: /^\s*(?:export\s+)?(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s*)?\(/, kind: "function" },
    { re: /^\s*(?:export\s+)?class\s+(\w+)/, kind: "class" },
  ],
  typescript: [
    { re: /^\s*(?:export\s+)?(?:async\s+)?function\s*\*?\s*(\w+)/, kind: "function" },
    { re: /^\s*(?:export\s+)?(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s*)?\(/, kind: "function" },
    { re: /^\s*(?:export\s+)?(?:abstract\s+)?class\s+(\w+)/, kind: "class" },
    { re: /^\s*(?:export\s+)?interface\s+(\w+)/, kind: "class" },
  ],
  python: [
    { re: /^\s*def\s+(\w+)/, kind: "function" },
    { re: /^\s*class\s+(\w+)/, kind: "class" },
  ],
  rust: [
    { re: /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)/, kind: "function" },
    { re: /^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait)\s+(\w+)/, kind: "class" },
  ],
  java: [
    { re: /^\s*(?:public|private|protected)?\s*[\w<>\[\],\s]+\s+(\w+)\s*\([^;]*\)\s*\{?\s*$/, kind: "function" },
    { re: /^\s*(?:public\s+)?(?:abstract\s+|final\s+)?class\s+(\w+)/, kind: "class" },
    { re: /^\s*(?:public\s+)?interface\s+(\w+)/, kind: "class" },
  ],
  c: [
    { re: /^\s*[\w*]+\s+(\w+)\s*\([^;]*\)\s*\{?\s*$/, kind: "function" },
    { re: /^\s*(?:typedef\s+)?struct\s+(\w+)/, kind: "class" },
  ],
  markdown: [{ re: /^(#{1,6})\s+(.+)/, kind: "heading" }],
  sql: [{ re: /^\s*CREATE\s+(?:TABLE|VIEW|FUNCTION)\s+(\w+)/i, kind: "class" }],
};

export interface SymbolItem {
  name: string;
  /** 1-based line number. */
  line: number;
  kind: SymbolPattern["kind"];
}

/**
 * Run the mode's patterns over the document lines; first match per line wins.
 * Unknown mode ids yield `[]` (the panel shows a hint instead).
 */
export function extractSymbols(
  lines: ReadonlyArray<{ n: number; text: string }>,
  modeId: string | null,
): SymbolItem[] {
  const patterns = modeId ? SYMBOL_PATTERNS[modeId] : undefined;
  if (!patterns) return [];
  const out: SymbolItem[] = [];
  for (const { n, text } of lines) {
    for (const p of patterns) {
      const m = p.re.exec(text);
      if (m) {
        out.push({ name: (m[2] ?? m[1] ?? m[0]).trim(), line: n, kind: p.kind });
        break;
      }
    }
  }
  return out;
}
