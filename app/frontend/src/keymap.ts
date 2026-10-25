// Keyboard-shortcut model (spec §8/§10, M3 子项目 C): canonical accelerator
// strings, validation, and merging the `config.toml` `[keymap]` overrides with
// the built-in defaults. Pure/DOM-free so every rule is unit-testable.

export type ActionId =
  | "open"
  | "save"
  | "save-as"
  | "query"
  | "jump"
  | "palette"
  | "close"
  | "sidebar"
  | "inspector"
  | "can-panel"
  | "can-import"
  | "can-parse"
  | "can-snippet";

/** Stable action order (also the modal's row order). */
export const ACTION_IDS: ActionId[] = [
  "open",
  "save",
  "save-as",
  "query",
  "jump",
  "palette",
  "close",
  "sidebar",
  "inspector",
  "can-panel",
  "can-import",
  "can-parse",
  "can-snippet",
];

/** Built-in bindings; actions absent here start unbound. */
export const DEFAULT_BINDINGS: Partial<Record<ActionId, string>> = {
  open: "Ctrl+O",
  save: "Ctrl+S",
  "save-as": "Ctrl+Shift+S",
  query: "Ctrl+Shift+F",
  jump: "Ctrl+G",
  palette: "Ctrl+Shift+P",
};

const NAMED_KEYS = [
  "ArrowUp",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "Backspace",
  "Delete",
  "Enter",
  "Tab",
  "Escape",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "Insert",
  "Space",
];

const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta"]);

export interface ParsedAccel {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  key: string;
}

function normalizeKey(k: string): string {
  if (k === " ") return "Space";
  if (/^[a-z]$/i.test(k)) return k.toUpperCase();
  if (/^f([1-9]|1[0-9]|2[0-4])$/i.test(k)) return k.toUpperCase();
  const named = NAMED_KEYS.find((n) => n.toLowerCase() === k.toLowerCase());
  return named ?? k;
}

/** Canonical accelerator for a keydown event; `null` for modifier-only presses. */
export function eventToAccel(e: {
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}): string | null {
  if (MODIFIER_KEYS.has(e.key)) return null;
  return formatAccel({
    ctrl: e.ctrlKey || e.metaKey, // Ctrl token matches Cmd on macOS
    alt: e.altKey,
    shift: e.shiftKey,
    key: normalizeKey(e.key),
  });
}

export function formatAccel(p: ParsedAccel): string {
  const parts: string[] = [];
  if (p.ctrl) parts.push("Ctrl");
  if (p.alt) parts.push("Alt");
  if (p.shift) parts.push("Shift");
  parts.push(p.key);
  return parts.join("+");
}

export function parseAccel(s: string): ParsedAccel | { error: string } {
  const text = s.trim();
  if (!text) return { error: "空绑定" };
  const parts = text.split("+").map((p) => p.trim()).filter(Boolean);
  if (parts.length === 0) return { error: "空绑定" };
  let ctrl = false;
  let alt = false;
  let shift = false;
  for (const m of parts.slice(0, -1)) {
    const lower = m.toLowerCase();
    if (lower === "ctrl" || lower === "control" || lower === "cmd" || lower === "meta") {
      ctrl = true;
    } else if (lower === "alt" || lower === "option") {
      alt = true;
    } else if (lower === "shift") {
      shift = true;
    } else {
      return { error: `未知修饰键 ${m}` };
    }
  }
  return { ctrl, alt, shift, key: normalizeKey(parts[parts.length - 1]) };
}

/** Canonical form, or `null` when the string does not parse. */
export function normalizeAccel(s: string): string | null {
  const p = parseAccel(s);
  return "error" in p ? null : formatAccel(p);
}

/**
 * Reject bindings that would hijack typing: a printable/control key needs Ctrl
 * or Alt; bare function keys are fine.
 */
export function validate(accel: string): { ok: true } | { ok: false; reason: string } {
  const p = parseAccel(accel);
  if ("error" in p) return { ok: false, reason: p.error };
  const isFn = /^F([1-9]|1[0-9]|2[0-4])$/.test(p.key);
  if (!p.ctrl && !p.alt && !isFn) {
    return { ok: false, reason: "需包含 Ctrl 或 Alt（或使用 F1–F24）" };
  }
  return { ok: true };
}

export interface MergedBindings {
  bindings: Partial<Record<ActionId, string>>;
  accelToAction: Partial<Record<string, ActionId>>;
  conflicts: { action: ActionId; other: ActionId; accel: string }[];
  unknown: string[];
}

/**
 * Defaults first, then overrides: `""` unbinds, unknown action ids are reported
 * and ignored, and a duplicate accelerator is claimed by whoever came first
 * (the later action stays unbound and lands in `conflicts`).
 */
export function mergeBindings(overrides: Record<string, string | undefined>): MergedBindings {
  const bindings: Partial<Record<ActionId, string>> = {};
  const accelToAction: Partial<Record<string, ActionId>> = {};
  const conflicts: { action: ActionId; other: ActionId; accel: string }[] = [];
  const unknown: string[] = [];

  for (const id of ACTION_IDS) {
    if (overrides[id] !== undefined) continue; // handled below
    const accel = DEFAULT_BINDINGS[id];
    if (!accel || accelToAction[accel]) continue;
    bindings[id] = accel;
    accelToAction[accel] = id;
  }
  for (const [rawId, accel] of Object.entries(overrides)) {
    if (!(ACTION_IDS as string[]).includes(rawId)) {
      unknown.push(rawId);
      continue;
    }
    if (accel === undefined || accel === "") continue; // default kept / unbound
    const id = rawId as ActionId;
    const owner = accelToAction[accel];
    if (owner && owner !== id) {
      conflicts.push({ action: id, other: owner, accel });
      continue;
    }
    bindings[id] = accel;
    accelToAction[accel] = id;
  }
  return { bindings, accelToAction, conflicts, unknown };
}

/** Which other action already owns `accel` (recording-time duplicate check). */
export function conflictOf(
  accel: string,
  bindings: Partial<Record<ActionId, string>>,
  selfId: ActionId,
): ActionId | null {
  for (const [id, a] of Object.entries(bindings)) {
    if (id !== selfId && a === accel) return id as ActionId;
  }
  return null;
}
