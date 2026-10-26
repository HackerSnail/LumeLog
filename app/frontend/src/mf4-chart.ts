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
