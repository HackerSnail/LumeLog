//! App-shell chrome: pane resizing + collapse persistence. The clamp math and
//! the storage reads/writes are kept free of layout coupling so `clampWidth` is
//! headless-tested; the pointer-drag wiring only ever runs under `tauri dev`.

export const MIN_SIDEBAR = 160;
export const MAX_SIDEBAR = 480;
export const MIN_INSPECTOR = 200;
export const MAX_INSPECTOR = 520;

/** Clamp a pane width to `[min, max]`; a non-finite input falls back to `min`. */
export function clampWidth(w: number, min: number, max: number): number {
  if (!Number.isFinite(w)) return min;
  return Math.min(max, Math.max(min, w));
}

function readNum(key: string): number | undefined {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return undefined;
    const n = Number.parseInt(raw, 10);
    return Number.isFinite(n) ? n : undefined;
  } catch {
    return undefined;
  }
}

function write(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* storage unavailable — the session still works, it just won't persist */
  }
}

export function loadFlag(key: string, defaultValue: boolean): boolean {
  try {
    const raw = localStorage.getItem(key);
    if (raw === null) return defaultValue;
    return raw === "1";
  } catch {
    return defaultValue;
  }
}

export function storeFlag(key: string, on: boolean): void {
  write(key, on ? "1" : "0");
}

export interface ResizableOpts {
  min: number;
  max: number;
  storageKey: string;
  /** "left" = pane on the left, grows rightward; "right" = mirror (inspector). */
  edge: "left" | "right";
  /** Resize is suppressed while the pane is collapsed. */
  collapsed: () => boolean;
}

/**
 * Make `handle` a horizontal drag splitter that sets `pane`'s inline width,
 * clamped and persisted. Restores the stored width on attach.
 */
export function makeResizable(
  pane: HTMLElement,
  handle: HTMLElement,
  opts: ResizableOpts,
): void {
  const apply = (px: number) => {
    const w = clampWidth(px, opts.min, opts.max);
    pane.style.width = `${w}px`;
    write(opts.storageKey, String(w));
  };
  const stored = readNum(opts.storageKey);
  if (stored !== undefined) apply(stored);

  handle.addEventListener("pointerdown", (e) => {
    if (opts.collapsed()) return;
    e.preventDefault();
    handle.classList.add("dragging");
    handle.setPointerCapture(e.pointerId);
    const anchorX = e.clientX;
    const startW = pane.getBoundingClientRect().width;
    const move = (ev: PointerEvent) => {
      const dx = ev.clientX - anchorX;
      apply(opts.edge === "left" ? startW + dx : startW - dx);
    };
    const up = () => {
      handle.classList.remove("dragging");
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  });
}
