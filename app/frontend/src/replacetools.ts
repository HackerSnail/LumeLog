//! Replace-with-preview rules (Plan 46): pure match/replace over a text
//! slice; the editor modal owns DOM and dispatches one change.

export type Mode = "literal" | "regex";

export interface MatchInfo {
  /** Absolute [from, to) into the scanned text. */
  from: number;
  to: number;
  /** 1-based line of `from`. */
  line: number;
  /** The match's whole line, capped at 200 chars. */
  preview: string;
}

export function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Global matcher for the pattern; literal mode escapes metacharacters. */
export function buildReplaceRe(
  find: string,
  mode: Mode,
  caseSensitive: boolean,
): RegExp | { error: string } {
  if (find === "") return { error: "查找内容为空" };
  const src = mode === "literal" ? escapeRegex(find) : find;
  try {
    return new RegExp(src, caseSensitive ? "g" : "gi");
  } catch (e) {
    return { error: `正则无效：${(e as Error).message}` };
  }
}

/** Every non-empty match with its line number and line preview. */
export function findMatches(text: string, re: RegExp): MatchInfo[] {
  const rx = new RegExp(re.source, re.flags.includes("g") ? re.flags : re.flags + "g");
  const out: MatchInfo[] = [];
  let line = 1;
  let lineStart = 0;
  // Lines are counted as the scan advances between matches — counting inside
  // the match alone misses newlines in the gaps.
  let scanned = 0;
  let m: RegExpExecArray | null;
  const capped = (s: string): string => (s.length > 200 ? s.slice(0, 200) + "…" : s);
  while ((m = rx.exec(text)) !== null) {
    if (m[0].length === 0) {
      rx.lastIndex++; // zero-width match: step over, do not record
      continue;
    }
    for (let i = scanned; i < m.index; i++) {
      if (text[i] === "\n") {
        line++;
        lineStart = i + 1;
      }
    }
    scanned = m.index + m[0].length;
    const lineEnd = text.indexOf("\n", m.index);
    const stop = lineEnd === -1 ? text.length : lineEnd;
    out.push({
      from: m.index,
      to: m.index + m[0].length,
      line,
      preview: capped(text.slice(lineStart, stop)),
    });
  }
  return out;
}

/** Apply the replacement over `text`. Literal mode treats `repl` verbatim
 *  (`$` has no meaning); regex mode keeps `$1` group references. */
export function applyReplace(text: string, re: RegExp, repl: string, mode: Mode): string {
  if (mode === "literal") return text.replace(re, () => repl);
  return text.replace(re, repl);
}

export type ReplacePreview =
  | { ok: true; matches: MatchInfo[]; result: string }
  | { ok: false; error: string };

/** Build + find + replace in one step over the given text slice. */
export function replacePreview(
  text: string,
  find: string,
  repl: string,
  mode: Mode,
  caseSensitive: boolean,
): ReplacePreview {
  const built = buildReplaceRe(find, mode, caseSensitive);
  if ("error" in built) return { ok: false, error: built.error };
  const matches = findMatches(text, built);
  return { ok: true, matches, result: applyReplace(text, built, repl, mode) };
}
