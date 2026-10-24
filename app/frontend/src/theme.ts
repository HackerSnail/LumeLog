//! Theme resolution + application (UI reference: NoteLume's design system).
//! Three presets keyed by `data-theme` on `<html>` — light / dark / vampire
//! (Dracula) — plus an "auto" *preference* that follows the OS
//! `prefers-color-scheme`. The stored value is a preference; the value painted
//! onto the DOM is the resolved concrete theme. `resolveTheme` is the pure,
//! headless-tested core; everything touching `localStorage` / `matchMedia` /
//! the DOM lives in the side-effecting functions below.

export type ThemeName = "light" | "dark" | "vampire";
export type ThemePref = ThemeName | "auto";

const STORAGE_KEY = "lumelog.theme";
/** Cycle order for the toolbar toggle; "auto" wraps back to light. */
export const PREF_ORDER: readonly ThemePref[] = ["light", "dark", "vampire", "auto"];

/** Resolve a preference to a concrete theme given whether the system is dark. */
export function resolveTheme(pref: ThemePref, systemDark: boolean): ThemeName {
  if (pref === "auto") return systemDark ? "dark" : "light";
  return pref;
}

/** The next preference in the cycle, wrapping around. */
export function nextPref(pref: ThemePref): ThemePref {
  const i = PREF_ORDER.indexOf(pref);
  return PREF_ORDER[(i + 1 + PREF_ORDER.length) % PREF_ORDER.length] ?? "auto";
}

function isPref(v: string | null): v is ThemePref {
  return v === "light" || v === "dark" || v === "vampire" || v === "auto";
}

/** Read the stored preference; anything unrecognised (or storage failure) → auto. */
export function loadPref(): ThemePref {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (isPref(v)) return v;
  } catch {
    /* storage unavailable (privacy mode) — fall through to default */
  }
  return "auto";
}

export function savePref(pref: ThemePref): void {
  try {
    localStorage.setItem(STORAGE_KEY, pref);
  } catch {
    /* non-fatal: the session still themes correctly, it just won't persist */
  }
}

/** Write the resolved theme onto `<html data-theme>`; CSS tokens follow it. */
export function paint(name: ThemeName): void {
  document.documentElement.dataset.theme = name;
}

export interface ThemeController {
  /** The concrete theme currently painted. */
  current: () => ThemeName;
  /** The stored preference (may be "auto"). */
  pref: () => ThemePref;
  set: (pref: ThemePref) => void;
  cycle: () => void;
}

/**
 * Install the theme controller: paint immediately, repaint when the OS scheme
 * flips while on "auto", and expose set/cycle for the toolbar + command palette.
 */
export function initTheme(): ThemeController {
  const media =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;
  let pref = loadPref();
  let name = resolveTheme(pref, media?.matches ?? false);
  paint(name);

  const reapply = () => {
    name = resolveTheme(pref, media?.matches ?? false);
    paint(name);
  };
  media?.addEventListener?.("change", reapply);

  const set = (p: ThemePref) => {
    pref = p;
    savePref(p);
    reapply();
  };
  return { current: () => name, pref: () => pref, set, cycle: () => set(nextPref(pref)) };
}
