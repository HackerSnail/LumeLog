//! Fuzzy command palette (UI reference: NoteLume's Ctrl+Shift+P). The scoring
//! and filtering are pure and headless-tested; the overlay DOM is built in
//! `main.ts` and only exercised under `tauri dev`.

export interface Command {
  id: string;
  /** Primary label; this is what the query is fuzzy-matched against. */
  title: string;
  /** Right-aligned secondary text (shortcut or category). */
  hint?: string;
  run: () => void;
}

/**
 * Case-insensitive subsequence score. Returns `null` when `query` is not a
 * subsequence of `text` (no match), else a number where higher = better.
 * Bonuses for prefix and consecutive runs, and tightens on fewer skipped
 * chars, so "sav" ranks "保存 / Save" above a scattered match.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (q === "") return 0;
  let ti = 0;
  let score = 0;
  let prev = -2;
  for (let qi = 0; qi < q.length; qi++) {
    const found = t.indexOf(q[qi], ti);
    if (found === -1) return null;
    score += found === prev + 1 ? 3 : 1; // run bonus
    if (found === 0) score += 4; // start-of-text bonus
    prev = found;
    ti = found + 1;
  }
  return score - (ti - q.length) * 0.1 - t.length * 0.01;
}

/** Commands whose title fuzzy-matches `query`, best first. Empty query → all,
 *  in the given order (a deliberate browse-the-whole-list mode). */
export function filterCommands(
  query: string,
  commands: readonly Command[],
): Command[] {
  if (query.trim() === "") return [...commands];
  const scored: { c: Command; s: number }[] = [];
  for (const c of commands) {
    const s = fuzzyScore(query, c.title);
    if (s !== null) scored.push({ c, s });
  }
  scored.sort((a, b) => b.s - a.s);
  return scored.map((x) => x.c);
}
