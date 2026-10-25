import { describe, expect, it } from "vitest";
import {
  DEFAULT_BINDINGS,
  conflictOf,
  eventToAccel,
  formatAccel,
  mergeBindings,
  normalizeAccel,
  parseAccel,
  validate,
} from "./keymap";

const ev = (key: string, mods: Partial<{ ctrl: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) => ({
  key,
  ctrlKey: mods.ctrl ?? false,
  altKey: mods.alt ?? false,
  shiftKey: mods.shift ?? false,
  metaKey: mods.meta ?? false,
});

describe("eventToAccel", () => {
  it("normalizes modifiers and keys (Ctrl == Cmd)", () => {
    expect(eventToAccel(ev("s", { ctrl: true }))).toBe("Ctrl+S");
    expect(eventToAccel(ev("s", { meta: true }))).toBe("Ctrl+S");
    expect(eventToAccel(ev("F3", { alt: true, shift: true }))).toBe("Alt+Shift+F3");
    expect(eventToAccel(ev(" ", { ctrl: true }))).toBe("Ctrl+Space");
    expect(eventToAccel(ev("+", { ctrl: true }))).toBe("Ctrl+Plus");
    expect(eventToAccel(ev("ArrowUp", { ctrl: true }))).toBe("Ctrl+ArrowUp");
  });

  it("returns null for modifier-only presses", () => {
    expect(eventToAccel(ev("Control", { ctrl: true }))).toBeNull();
    expect(eventToAccel(ev("Shift", { shift: true }))).toBeNull();
    expect(eventToAccel(ev("Meta", { meta: true }))).toBeNull();
  });
});

describe("parse/format/validate", () => {
  it("normalizes order and case", () => {
    expect(normalizeAccel("shift+ctrl+f")).toBe("Ctrl+Shift+F");
    expect(normalizeAccel("Cmd+F")).toBe("Ctrl+F");
    expect(normalizeAccel("f3")).toBe("F3");
    const p = parseAccel("Alt+Shift+F3");
    expect("error" in p).toBe(false);
    if (!("error" in p)) expect(formatAccel(p)).toBe("Alt+Shift+F3");
  });

  it("rejects bindings that would hijack typing or shadow context keys", () => {
    expect(validate("S")).toEqual({ ok: false, reason: "需包含 Ctrl 或 Alt（或使用 F1–F24）" });
    expect(validate("Escape").ok).toBe(false);
    expect(validate("").ok).toBe(false);
    // F3 (any mods) and plain Ctrl+F are fixed context keys.
    expect(validate("F3").ok).toBe(false);
    expect(validate("Shift+F3").ok).toBe(false);
    expect(validate("Ctrl+F").ok).toBe(false);
    expect(validate("F5")).toEqual({ ok: true });
    expect(validate("Ctrl+Shift+F")).toEqual({ ok: true });
    expect(validate("Alt+1")).toEqual({ ok: true });
    expect(validate("Ctrl+Plus")).toEqual({ ok: true });
  });
});

describe("mergeBindings", () => {
  it("applies defaults, overrides and explicit unbinds", () => {
    const merged = mergeBindings({ save: "Ctrl+K", close: "Ctrl+F4", open: "" });
    expect(merged.bindings.save).toBe("Ctrl+K");
    expect(merged.bindings.close).toBe("Ctrl+F4");
    expect(merged.bindings.open).toBeUndefined();
    expect(merged.accelToAction["Ctrl+O"]).toBeUndefined();
    expect(merged.accelToAction["Ctrl+Shift+P"]).toBe("palette");
    expect(merged.conflicts).toEqual([]);
    expect(merged.unknown).toEqual([]);
    expect(merged.invalid).toEqual([]);
  });

  it("drops invalid file values and keeps the default", () => {
    const bare = mergeBindings({ save: "S" });
    expect(bare.invalid).toEqual(["save"]);
    expect(bare.bindings.save).toBe("Ctrl+S"); // default survives
    expect(bare.accelToAction["S"]).toBeUndefined();

    const reserved = mergeBindings({ save: "F3" });
    expect(reserved.invalid).toEqual(["save"]);
    expect(reserved.bindings.save).toBe("Ctrl+S");
  });

  it("canonicalizes file values before claiming", () => {
    const ok = mergeBindings({ save: "ctrl+k" });
    expect(ok.bindings.save).toBe("Ctrl+K");
    expect(ok.accelToAction["Ctrl+K"]).toBe("save");

    const clash = mergeBindings({ save: "ctrl+o" });
    expect(clash.conflicts).toEqual([{ action: "save", other: "open", accel: "Ctrl+O" }]);
    expect(clash.bindings.save).toBeUndefined();
  });

  it("first binding wins on a duplicate (defaults first)", () => {
    const merged = mergeBindings({ save: "Ctrl+O" });
    // `open` claimed Ctrl+O first; the save override is dropped and reported.
    expect(merged.bindings.open).toBe("Ctrl+O");
    expect(merged.bindings.save).toBeUndefined();
    expect(merged.accelToAction["Ctrl+O"]).toBe("open");
    expect(merged.conflicts).toEqual([{ action: "save", other: "open", accel: "Ctrl+O" }]);
  });

  it("reports unknown action ids", () => {
    const merged = mergeBindings({ nope: "Ctrl+J" });
    expect(merged.unknown).toEqual(["nope"]);
    expect(merged.accelToAction["Ctrl+J"]).toBeUndefined();
  });

  it("keeps every default when there are no overrides", () => {
    const merged = mergeBindings({});
    for (const [id, accel] of Object.entries(DEFAULT_BINDINGS)) {
      expect(merged.bindings[id as keyof typeof merged.bindings]).toBe(accel);
    }
  });
});

describe("conflictOf", () => {
  it("finds the other owner and excludes self", () => {
    const bindings = mergeBindings({}).bindings;
    expect(conflictOf("Ctrl+O", bindings, "save")).toBe("open");
    expect(conflictOf("Ctrl+O", bindings, "open")).toBeNull();
    expect(conflictOf("Ctrl+J", bindings, "save")).toBeNull();
  });
});
