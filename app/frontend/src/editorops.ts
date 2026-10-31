// Line/case transforms for the editor parity commands (Plan 27) — pure and
// DOM-free so every rule is unit-testable. `transformLines` in editor.ts turns
// these into a single undoable dispatch over the selection's line range.

export type LineOp =
  | "sort-asc"
  | "sort-desc"
  | "dedupe"
  | "reverse"
  | "upper"
  | "lower"
  | "trim"
  | "drop-empty"
  | "join";

/** Apply one line operation; input lines are NOT mutated. */
export function applyLineOp(lines: string[], op: LineOp): string[] {
  switch (op) {
    case "sort-asc":
      return [...lines].sort((a, b) => a.localeCompare(b));
    case "sort-desc":
      return [...lines].sort((a, b) => b.localeCompare(a));
    case "dedupe": {
      const seen = new Set<string>();
      const out: string[] = [];
      for (const l of lines) {
        if (!seen.has(l)) {
          seen.add(l);
          out.push(l);
        }
      }
      return out;
    }
    case "reverse":
      return [...lines].reverse();
    case "upper":
      return lines.map((l) => l.toUpperCase());
    case "lower":
      return lines.map((l) => l.toLowerCase());
    case "trim":
      return lines.map((l) => l.trim());
    case "drop-empty":
      return lines.filter((l) => l.trim() !== "");
    case "join":
      return [lines.map((l) => l.trim()).filter((l) => l !== "").join(" ")];
  }
}

/** Editor font size bounds (Plan 27 zoom); N++-like ±1px steps. */
export const FONT_MIN = 9;
export const FONT_MAX = 28;
export const FONT_BASE = 14;

export function clampFontSize(px: number): number {
  if (!Number.isFinite(px)) return FONT_BASE;
  return Math.min(FONT_MAX, Math.max(FONT_MIN, Math.round(px)));
}
