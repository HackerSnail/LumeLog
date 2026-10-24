// CanPanel view-filter engine (spec §7.4). Pure predicate over decoded frames:
// it never mutates the input array or its frame objects, and an unset axis is
// simply not applied. All active axes combine with AND. This is the headless,
// fully-testable half of the filter; the toolbar that produces a `CanFilter`
// lives in canpanel.ts (Task 6).

import type { FrameJson } from "./ipc";

export interface CanFilter {
  /** Inclusive lower bound on `canId`. */
  idFrom?: number;
  /** Inclusive upper bound on `canId`. `idFrom > idTo` matches nothing. */
  idTo?: number;
  /** Exact message name (frames without a `msgName` are dropped when set). */
  msgName?: string;
  /** Signal condition: only active together with `signalOp` + `signalValue`. */
  signalName?: string;
  signalOp?: ">" | "<" | ">=" | "<=" | "==";
  signalValue?: number;
  /** Inclusive time window on `timestampMs` (frames without one are dropped). */
  timeFromMs?: number;
  timeToMs?: number;
}

function matchesId(canId: number, f: CanFilter): boolean {
  if (f.idFrom !== undefined && canId < f.idFrom) return false;
  if (f.idTo !== undefined && canId > f.idTo) return false;
  // Reversed closed range can never hold; short-circuit so it matches nothing.
  if (f.idFrom !== undefined && f.idTo !== undefined && f.idFrom > f.idTo) return false;
  return true;
}

function matchesSignal(frame: FrameJson, f: CanFilter): boolean {
  if (f.signalName === undefined || f.signalOp === undefined || f.signalValue === undefined) {
    return true;
  }
  const hit = frame.signals.find((s) => s.name === f.signalName);
  if (hit === undefined) return false;
  switch (f.signalOp) {
    case ">":
      return hit.physical > f.signalValue;
    case "<":
      return hit.physical < f.signalValue;
    case ">=":
      return hit.physical >= f.signalValue;
    case "<=":
      return hit.physical <= f.signalValue;
    case "==":
      // Pinned exact float semantics (no epsilon) — Object.is also rejects NaN.
      return Object.is(hit.physical, f.signalValue);
  }
}

function matchesTime(frame: FrameJson, f: CanFilter): boolean {
  if (f.timeFromMs === undefined && f.timeToMs === undefined) return true;
  if (frame.timestampMs === undefined) return false;
  if (f.timeFromMs !== undefined && frame.timestampMs < f.timeFromMs) return false;
  if (f.timeToMs !== undefined && frame.timestampMs > f.timeToMs) return false;
  return true;
}

/** Filter `frames` by `f` without mutating the input or its elements. */
export function applyCanFilter(frames: FrameJson[], f: CanFilter): FrameJson[] {
  return frames.filter(
    (frame) =>
      matchesId(frame.canId, f) &&
      (f.msgName === undefined || frame.msgName === f.msgName) &&
      matchesSignal(frame, f) &&
      matchesTime(frame, f),
  );
}
