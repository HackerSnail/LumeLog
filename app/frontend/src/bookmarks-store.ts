//! Cross-session bookmark persistence (Plan 38): per-file-path line numbers.
//! Pure store rules; storage I/O lives in `main.ts`. Line numbers — not char
//! offsets — survive between sessions because disk edits invalidate offsets.

export const BOOKMARKS_PER_DOC_MAX = 200;

/** path → 1-based line numbers, ascending, deduped. */
export type BookmarkStore = Record<string, number[]>;

export function serializeBookmarkStore(store: BookmarkStore): string {
  return JSON.stringify(store);
}

/** Tolerant parse: bad entries dropped per document, caps enforced. */
export function parseBookmarkStore(json: string | null): BookmarkStore {
  if (!json) return {};
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return {};
  }
  if (!raw || typeof raw !== "object") return {};
  const out: BookmarkStore = {};
  for (const [path, lines] of Object.entries(raw as Record<string, unknown>)) {
    if (path === "" || !Array.isArray(lines)) continue;
    const clean: number[] = [];
    for (const l of lines) {
      if (typeof l !== "number" || !Number.isInteger(l) || l < 1) continue;
      if (!clean.includes(l)) clean.push(l);
      if (clean.length >= BOOKMARKS_PER_DOC_MAX) break;
    }
    clean.sort((a, b) => a - b);
    if (clean.length > 0) out[path] = clean;
  }
  return out;
}
