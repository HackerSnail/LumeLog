import { describe, expect, it } from "vitest";
import {
  conflictMacro,
  macroForAccel,
  parseMacroBindings,
  serializeMacroBindings,
} from "./macrokeys";

describe("parseMacroBindings", () => {
  it("returns {} for null/corrupt input", () => {
    expect(parseMacroBindings(null)).toEqual({});
    expect(parseMacroBindings("{oops")).toEqual({});
    expect(parseMacroBindings("42")).toEqual({});
  });

  it("re-validates accels and drops empties/dups", () => {
    const json = JSON.stringify({
      "加前缀": "Ctrl+Shift+1", // ok
      "整理": "F3", // reserved → dropped
      "": "Ctrl+9", // empty name → dropped
      "裸键": "A", // needs Ctrl/Alt → dropped
      "后到的": "Ctrl+Shift+1", // dup accel → first macro wins
      "坏值": 42,
    });
    expect(parseMacroBindings(json)).toEqual({ 加前缀: "Ctrl+Shift+1" });
  });

  it("round-trips through serialize", () => {
    const b = { 加前缀: "Ctrl+Shift+1", 整理: "F12" };
    expect(parseMacroBindings(serializeMacroBindings(b))).toEqual(b);
  });
});

describe("conflictMacro / macroForAccel", () => {
  const b = { a: "Ctrl+1", b: "Ctrl+2" };
  it("finds the other macro owning an accel, excluding self", () => {
    expect(conflictMacro("Ctrl+1", b, "b")).toBe("a");
    expect(conflictMacro("Ctrl+1", b, "a")).toBeNull();
    expect(conflictMacro("Ctrl+9", b, "a")).toBeNull();
  });
  it("maps an accel back to its macro", () => {
    expect(macroForAccel("Ctrl+2", b)).toBe("b");
    expect(macroForAccel("Ctrl+3", b)).toBeNull();
  });
});
