//! Document tags keyed by file path (Plan 33) — pure store rules, DOM-free.
//! Storage I/O and the inspector chips live in `main.ts`.

export const TAG_MAX_LEN = 32;
export const TAGS_PER_DOC_MAX = 20;

/** path → tags, insertion order preserved. */
export type TagStore = Record<string, string[]>;

export function normalizeTag(raw: string): string | null {
  const t = raw.trim();
  if (t === "") return null;
  return t.slice(0, TAG_MAX_LEN);
}

/** Insert `tag` under `path`; pure — returns a new store on success. */
export function addTag(
  store: TagStore,
  path: string,
  raw: string,
): { store: TagStore; tag: string | null; reason?: "dup" | "full" } {
  const tag = normalizeTag(raw);
  if (!tag) return { store, tag: null };
  const tags = store[path] ?? [];
  if (tags.includes(tag)) return { store, tag: null, reason: "dup" };
  if (tags.length >= TAGS_PER_DOC_MAX) return { store, tag: null, reason: "full" };
  return { store: { ...store, [path]: [...tags, tag] }, tag };
}

export function removeTag(store: TagStore, path: string, tag: string): TagStore {
  const tags = store[path];
  if (!tags) return store;
  const next = tags.filter((t) => t !== tag);
  if (next.length === tags.length) return store;
  const copy = { ...store };
  if (next.length === 0) delete copy[path];
  else copy[path] = next;
  return copy;
}

export function serializeTagStore(store: TagStore): string {
  return JSON.stringify(store);
}

/** Tolerant parse: bad entries dropped per document, caps enforced. */
export function parseTagStore(json: string | null): TagStore {
  if (!json) return {};
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return {};
  }
  if (!raw || typeof raw !== "object") return {};
  const out: TagStore = {};
  for (const [path, tags] of Object.entries(raw as Record<string, unknown>)) {
    if (path === "" || !Array.isArray(tags)) continue;
    const clean: string[] = [];
    for (const t of tags) {
      if (typeof t !== "string") continue;
      const tag = normalizeTag(t);
      if (tag && !clean.includes(tag)) clean.push(tag);
      if (clean.length >= TAGS_PER_DOC_MAX) break;
    }
    if (clean.length > 0) out[path] = clean;
  }
  return out;
}
