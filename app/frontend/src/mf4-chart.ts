// Waveform math and rendering for the MF4 dock (Roadmap-4 increment 2).
//
// The math is written as pure functions so it can be tested without a canvas;
// `Mf4Chart` (below) only turns their results into pixels. The backend already
// reduced samples to per-bucket min/max envelopes, so drawing never touches raw
// signals.

export interface Win {
  start: number;
  end: number;
}

/** Narrowest zoom window is 1/1e6 of the full span, and never a zero-width window. */
const MIN_SPAN_FRACTION = 1e6;

/** Clamp a window to `full`, enforcing a positive width and the minimum span. */
export function clampWindow(win: Win, full: Win): Win {
  const fullSpan = full.end - full.start;
  const minSpan = Math.max(fullSpan / MIN_SPAN_FRACTION, 1e-6);
  let start = win.start;
  let end = win.end;
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) {
    return { start: full.start, end: Math.min(full.start + minSpan, full.end) };
  }
  if (end - start < minSpan) {
    const mid = (start + end) / 2;
    start = mid - minSpan / 2;
    end = mid + minSpan / 2;
  }
  if (end - start > fullSpan) {
    return { start: full.start, end: full.end };
  }
  if (start < full.start) {
    const d = full.start - start;
    start += d;
    end += d;
  }
  if (end > full.end) {
    const d = end - full.end;
    start -= d;
    end -= d;
  }
  return { start: Math.max(full.start, start), end: Math.min(full.end, end) };
}

/** Zoom around `anchorT` (the time under the cursor); the anchor stays put. */
export function zoomAt(win: Win, factor: number, anchorT: number, full: Win): Win {
  const span = win.end - win.start;
  const ratio = (anchorT - win.start) / span;
  const nextSpan = span * factor;
  const start = anchorT - ratio * nextSpan;
  return clampWindow({ start, end: start + nextSpan }, full);
}

export function panBy(win: Win, dtSec: number, full: Win): Win {
  return clampWindow({ start: win.start + dtSec, end: win.end + dtSec }, full);
}

export function bucketIndex(t: number, win: Win, buckets: number): number {
  if (buckets <= 0) return 0;
  const span = win.end - win.start;
  if (!(span > 0)) return 0; // degenerate window: fold everything into bucket 0
  const frac = (t - win.start) / span;
  return Math.min(buckets - 1, Math.max(0, Math.floor(frac * buckets)));
}

/** Nearest non-empty bucket to `t`; `-1` when every bucket is empty. */
export function nearestBucket(times: (number | null)[], t: number): number {
  let best = -1;
  let bestDist = Infinity;
  for (let i = 0; i < times.length; i += 1) {
    const time = times[i];
    if (time === null || time === undefined) continue;
    const d = Math.abs(time - t);
    if (d < bestDist) {
      best = i;
      bestDist = d;
    }
  }
  return best;
}

export function xOf(t: number, win: Win, left: number, width: number): number {
  return left + ((t - win.start) / (win.end - win.start)) * width;
}

/** Normalized y: each series is drawn against its own [min,max] over the window. */
export function yOf(v: number, min: number, max: number, top: number, height: number): number {
  if (!Number.isFinite(v)) return Number.NaN;
  if (!(max > min)) return top + height / 2; // flat series sits on the midline
  return top + (1 - (v - min) / (max - min)) * height;
}

import type { Mf4Series } from "./ipc";

const PALETTE = [
  "#4e79a7",
  "#f28e2b",
  "#59a14f",
  "#e15759",
  "#b07aa1",
  "#76b7b2",
  "#ff9da7",
  "#9c755f",
  "#bab0ac",
  "#edc948",
];
const PAD = { left: 10, right: 10, top: 8, bottom: 20 };

/** Two-layer canvas: the static envelope is redrawn only on data/window changes;
 *  the hover crosshair and tooltip live on the transparent overlay.
 *  A missing 2d context (jsdom) degrades to "callbacks only, nothing drawn". */
export class Mf4Chart {
  private readonly host: HTMLElement;
  private readonly base: HTMLCanvasElement;
  private readonly overlay: HTMLCanvasElement;
  private readonly tooltip: HTMLDivElement;
  private readonly baseCtx: CanvasRenderingContext2D | null;
  private readonly overlayCtx: CanvasRenderingContext2D | null;
  private data: Mf4Series | null = null;
  private full: Win = { start: 0, end: 1 };
  private win: Win = { start: 0, end: 1 };
  private windowCb: ((w: Win) => void) | null = null;
  private hoverT: number | null = null;
  private busy = false;
  private dragging: { x: number; from: Win } | null = null;
  private raf = 0;
  private emitTimer: number | null = null;
  private readonly resizeObserver: ResizeObserver | null;

  private readonly onWheel = (e: WheelEvent): void => {
    e.preventDefault();
    const rect = this.base.getBoundingClientRect();
    const plotW = Math.max(1, rect.width - PAD.left - PAD.right);
    const frac = Math.min(1, Math.max(0, (e.clientX - rect.left - PAD.left) / plotW));
    const anchor = this.win.start + frac * (this.win.end - this.win.start);
    const factor = Math.pow(1.25, -e.deltaY / 100);
    this.setWindow(zoomAt(this.win, factor, anchor, this.full));
    this.scheduleEmit();
  };

  private readonly onDown = (e: PointerEvent): void => {
    if (e.button !== 0) return;
    this.dragging = { x: e.clientX, from: { ...this.win } };
    this.overlay.setPointerCapture?.(e.pointerId);
  };

  private readonly onMove = (e: PointerEvent): void => {
    const rect = this.base.getBoundingClientRect();
    if (this.dragging) {
      const plotW = Math.max(1, rect.width - PAD.left - PAD.right);
      const dt = (-(e.clientX - this.dragging.x) / plotW) * (this.dragging.from.end - this.dragging.from.start);
      this.setWindow(panBy(this.dragging.from, dt, this.full));
      this.scheduleEmit();
      return;
    }
    const plotW = Math.max(1, rect.width - PAD.left - PAD.right);
    const frac = Math.min(1, Math.max(0, (e.clientX - rect.left - PAD.left) / plotW));
    this.hoverT = this.win.start + frac * (this.win.end - this.win.start);
    this.drawOverlay();
  };

  private readonly onUp = (): void => {
    this.dragging = null;
  };

  private readonly onLeave = (): void => {
    this.hoverT = null;
    this.drawOverlay();
  };

  private readonly onDbl = (): void => {
    this.setWindow({ ...this.full });
    this.scheduleEmit();
  };

  constructor(host: HTMLElement) {
    this.host = host;
    host.classList.add("mf4-chart");
    this.base = document.createElement("canvas");
    this.base.className = "mf4-chart-base";
    this.overlay = document.createElement("canvas");
    this.overlay.className = "mf4-chart-overlay";
    this.tooltip = document.createElement("div");
    this.tooltip.className = "mf4-chart-tip";
    this.tooltip.style.display = "none";
    host.append(this.base, this.overlay, this.tooltip);
    this.baseCtx = this.base.getContext("2d");
    this.overlayCtx = this.overlay.getContext("2d");
    this.overlay.addEventListener("wheel", this.onWheel, { passive: false });
    this.overlay.addEventListener("pointerdown", this.onDown);
    this.overlay.addEventListener("pointermove", this.onMove);
    this.overlay.addEventListener("pointerup", this.onUp);
    this.overlay.addEventListener("pointerleave", this.onLeave);
    this.overlay.addEventListener("dblclick", this.onDbl);
    this.resizeObserver =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => this.resize());
    this.resizeObserver?.observe(host);
    this.resize();
  }

  setData(data: Mf4Series): void {
    this.data = data;
    this.full = { start: data.fromSec, end: data.toSec > data.fromSec ? data.toSec : data.fromSec + 1 };
    this.win = { ...this.full };
    this.scheduleDraw();
  }

  setWindow(win: Win): void {
    this.win = clampWindow(win, this.full);
    this.scheduleDraw();
  }

  get window(): Win {
    return { ...this.win };
  }

  setBusy(busy: boolean): void {
    this.busy = busy;
    this.scheduleDraw();
  }

  onWindowChange(cb: (w: Win) => void): void {
    this.windowCb = cb;
  }

  destroy(): void {
    this.overlay.removeEventListener("wheel", this.onWheel);
    this.overlay.removeEventListener("pointerdown", this.onDown);
    this.overlay.removeEventListener("pointermove", this.onMove);
    this.overlay.removeEventListener("pointerup", this.onUp);
    this.overlay.removeEventListener("pointerleave", this.onLeave);
    this.overlay.removeEventListener("dblclick", this.onDbl);
    this.resizeObserver?.disconnect();
    if (this.emitTimer !== null) window.clearTimeout(this.emitTimer);
    if (this.raf) cancelAnimationFrame(this.raf);
    this.host.textContent = "";
    this.host.classList.remove("mf4-chart");
  }

  private scheduleEmit(): void {
    if (this.emitTimer !== null) return;
    this.emitTimer = window.setTimeout(() => {
      this.emitTimer = null;
      this.windowCb?.(this.window);
    }, 60);
  }

  private resize(): void {
    const dpr = window.devicePixelRatio || 1;
    const w = this.host.clientWidth || 600;
    const h = this.host.clientHeight || 240;
    for (const c of [this.base, this.overlay]) {
      c.width = Math.round(w * dpr);
      c.height = Math.round(h * dpr);
      c.style.width = `${w}px`;
      c.style.height = `${h}px`;
    }
    this.baseCtx?.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.overlayCtx?.setTransform(dpr, 0, 0, dpr, 0, 0);
    this.scheduleDraw();
  }

  private scheduleDraw(): void {
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.drawBase();
      this.drawOverlay();
    });
  }

  private plot(): { left: number; top: number; width: number; height: number } {
    const w = this.host.clientWidth || 600;
    const h = this.host.clientHeight || 240;
    return {
      left: PAD.left,
      top: PAD.top,
      width: Math.max(1, w - PAD.left - PAD.right),
      height: Math.max(1, h - PAD.top - PAD.bottom),
    };
  }

  private drawBase(): void {
    const ctx = this.baseCtx;
    if (!ctx) return;
    const p = this.plot();
    ctx.clearRect(0, 0, p.width + PAD.left + PAD.right, p.height + PAD.top + PAD.bottom);
    const style = getComputedStyle(this.host);
    const grid = style.getPropertyValue("--border") || "#ddd";
    const muted = style.getPropertyValue("--muted") || "#888";
    ctx.strokeStyle = grid;
    ctx.lineWidth = 1;
    ctx.strokeRect(p.left, p.top, p.width, p.height);
    for (let i = 1; i < 4; i += 1) {
      const y = p.top + (p.height * i) / 4;
      ctx.beginPath();
      ctx.moveTo(p.left, y);
      ctx.lineTo(p.left + p.width, y);
      ctx.stroke();
    }
    ctx.fillStyle = muted;
    ctx.font = "10px sans-serif";
    for (let i = 0; i <= 5; i += 1) {
      const t = this.win.start + ((this.win.end - this.win.start) * i) / 5;
      const x = xOf(t, this.win, p.left, p.width);
      ctx.fillText(formatSeconds(t), x + 2, p.top + p.height + 14);
    }
    if (this.busy) {
      ctx.fillText("读取中…", p.left + 6, p.top + 14);
    }
    if (!this.data) return;
    const n = this.data.times.length;
    this.data.series.forEach((s, si) => {
      let lo = Infinity;
      let hi = -Infinity;
      for (let i = 0; i < n; i += 1) {
        const bucketLo = s.min[i];
        const bucketHi = s.max[i];
        if (bucketLo !== null && bucketLo !== undefined) lo = Math.min(lo, bucketLo);
        if (bucketHi !== null && bucketHi !== undefined) hi = Math.max(hi, bucketHi);
      }
      if (!Number.isFinite(lo) || !Number.isFinite(hi)) return;
      ctx.strokeStyle = PALETTE[si % PALETTE.length];
      ctx.globalAlpha = 0.9;
      let prevX = Number.NaN;
      let prevY = Number.NaN;
      for (let i = 0; i < n; i += 1) {
        const mid = this.data!.times[i];
        if (mid === null || mid === undefined) continue;
        const x = xOf(mid, this.win, p.left, p.width);
        const tiny = yOf(s.max[i] ?? s.min[i] ?? lo, lo, hi, p.top, p.height);
        const high = yOf(s.min[i] ?? s.max[i] ?? hi, lo, hi, p.top, p.height);
        ctx.beginPath();
        ctx.moveTo(x, high);
        ctx.lineTo(x, tiny);
        ctx.stroke();
        if (Number.isFinite(prevX)) {
          ctx.globalAlpha = 0.35;
          ctx.beginPath();
          ctx.moveTo(prevX, prevY);
          ctx.lineTo(x, tiny);
          ctx.stroke();
          ctx.globalAlpha = 0.9;
        }
        prevX = x;
        prevY = tiny;
      }
    });
    ctx.globalAlpha = 1;
  }

  private drawOverlay(): void {
    const ctx = this.overlayCtx;
    if (!ctx) return;
    const p = this.plot();
    ctx.clearRect(0, 0, p.width + PAD.left + PAD.right, p.height + PAD.top + PAD.bottom);
    if (this.hoverT === null || !this.data) {
      this.tooltip.style.display = "none";
      return;
    }
    const x = xOf(this.hoverT, this.win, p.left, p.width);
    ctx.strokeStyle = "#888";
    ctx.setLineDash([3, 3]);
    ctx.beginPath();
    ctx.moveTo(x, p.top);
    ctx.lineTo(x, p.top + p.height);
    ctx.stroke();
    ctx.setLineDash([]);
    const b = nearestBucket(this.data.times, this.hoverT);
    const lines = [`t=${formatSeconds(this.hoverT)}`];
    if (b >= 0) {
      for (const s of this.data.series) {
        const v = s.first[b];
        lines.push(`${s.name}${s.unit ? ` (${s.unit})` : ""}: ${v === null || v === undefined ? "—" : v.toPrecision(6)}`);
      }
    }
    this.tooltip.textContent = lines.join("\n");
    this.tooltip.style.display = "block";
    this.tooltip.style.left = `${Math.min(x + 8, p.left + p.width - 160)}px`;
    this.tooltip.style.top = `${p.top + 6}px`;
  }
}

function formatSeconds(t: number): string {
  const abs = Math.abs(t);
  if (abs >= 100 || t === 0) return `${t.toFixed(1)}s`;
  if (abs >= 1) return `${t.toFixed(2)}s`;
  return `${(t * 1000).toFixed(1)}ms`;
}
