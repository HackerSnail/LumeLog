//! Macro keyboard bindings (Plan 41): macro name → canonical accelerator.
//! Pure rules; storage I/O in `main.ts`. These live in localStorage (macros
//! themselves are localStorage-scoped) — the config.toml `[keymap]` table is
//! action-id-scoped and would drop dynamic names as unknown.
import { normalizeAccel, validate } from "./keymap";

/** macro name → canonical accel. */
export type MacroBindings = Record<string, string>;

export function serializeMacroBindings(b: MacroBindings): string {
  return JSON.stringify(b);
}

/** Tolerant parse: accelerators re-validated (keymap rules), dup accels keep
 *  the first macro, empty names and invalid accels are dropped. */
export function parseMacroBindings(json: string | null): MacroBindings {
  if (!json) return {};
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return {};
  }
  if (!raw || typeof raw !== "object") return {};
  const out: MacroBindings = {};
  for (const [name, accel] of Object.entries(raw as Record<string, unknown>)) {
    if (name.trim() === "" || typeof accel !== "string") continue;
    const canon = normalizeAccel(accel);
    if (canon === null || !validate(canon).ok) continue;
    if (Object.values(out).includes(canon)) continue; // first macro wins
    out[name] = canon;
  }
  return out;
}

/** Which other macro already owns `accel` (recording-time duplicate check). */
export function conflictMacro(
  accel: string,
  bindings: MacroBindings,
  self: string,
): string | null {
  for (const [name, a] of Object.entries(bindings)) {
    if (name !== self && a === accel) return name;
  }
  return null;
}

export function macroForAccel(accel: string, bindings: MacroBindings): string | null {
  for (const [name, a] of Object.entries(bindings)) {
    if (a === accel) return name;
  }
  return null;
}
