import { describe, it, expect } from "vitest";
import { resolveTheme, nextPref, PREF_ORDER } from "./theme";

// Only the pure resolution/cycle logic is headless-testable; the localStorage /
// matchMedia / DOM side of `initTheme` is exercised in `tauri dev`.
describe("resolveTheme (three-theme + auto system)", () => {
  it("passes an explicit preset through unchanged regardless of system", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
    expect(resolveTheme("vampire", true)).toBe("vampire");
  });

  it("auto follows the system dark flag", () => {
    expect(resolveTheme("auto", true)).toBe("dark");
    expect(resolveTheme("auto", false)).toBe("light");
  });
});

describe("nextPref (toolbar cycle order)", () => {
  it("walks light → dark → vampire → auto → light", () => {
    expect(nextPref("light")).toBe("dark");
    expect(nextPref("dark")).toBe("vampire");
    expect(nextPref("vampire")).toBe("auto");
    expect(nextPref("auto")).toBe("light");
  });

  it("cycles every preference back to itself after a full turn", () => {
    for (const start of PREF_ORDER) {
      let p = start;
      for (let i = 0; i < PREF_ORDER.length; i++) p = nextPref(p);
      expect(p).toBe(start);
    }
  });
});
