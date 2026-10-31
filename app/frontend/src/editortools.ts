// Pure helpers for the editor tool trio (Plan 29): macro recording,
// overview-strip bucketing, and symbol extraction. DOM-free so every rule is
// unit-testable; the CodeMirror glue lives in editor.ts / main.ts.

// ---- macros ------------------------------------------------------------------

export type MacroOp =
  | { kind: "insert"; text: string }
  | { kind: "delete"; count: number };

/** Hard cap so a runaway recording cannot exhaust memory. */
export const MACRO_MAX_OPS = 10_000;

/**
 * Fold one transaction's changed ranges into macro ops, in document order.
 * Ranges come from `tr.changes.iterChangedRanges` — `removed` is the deleted
 * span length, `inserted` the replacing text.
 */
export function extractMacroOps(
  ranges: ReadonlyArray<{ removed: number; inserted: string }>,
): MacroOp[] {
  const ops: MacroOp[] = [];
  for (const r of ranges) {
    if (r.removed > 0) ops.push({ kind: "delete", count: r.removed });
    if (r.inserted.length > 0) ops.push({ kind: "insert", text: r.inserted });
  }
  return ops;
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
