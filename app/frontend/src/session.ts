//! Workspace session persistence (Plan 40): the list of open file paths plus
//! the active one, restored on startup. Pure rules; storage I/O in `main.ts`.

export const SESSION_DOCS_MAX = 50;

export interface SavedSession {
  /** Open file paths in tab order. */
  paths: string[];
  /** Path of the active tab; null when none or not in `paths`. */
  active: string | null;
}

export function serializeSession(s: SavedSession): string {
  return JSON.stringify(s);
}

/** Dedupe (order-preserving), drop empties, cap, and validate `active`. */
export function normalizeSession(raw: SavedSession): SavedSession {
  const paths: string[] = [];
  for (const p of raw.paths ?? []) {
    if (typeof p !== "string" || p === "" || paths.includes(p)) continue;
    if (paths.length >= SESSION_DOCS_MAX) break;
    paths.push(p);
  }
  const active =
    raw.active && paths.includes(raw.active) ? raw.active : null;
  return { paths, active };
}

/** Tolerant parse: corrupt or wrong-shaped input → empty session. */
export function parseSession(json: string | null): SavedSession {
  if (!json) return { paths: [], active: null };
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return { paths: [], active: null };
  }
  if (!raw || typeof raw !== "object") return { paths: [], active: null };
  const o = raw as { paths?: unknown; active?: unknown };
  return normalizeSession({
    paths: Array.isArray(o.paths) ? (o.paths as string[]) : [],
    active: typeof o.active === "string" ? o.active : null,
  });
}
