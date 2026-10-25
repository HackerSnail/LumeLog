// Crash-recovery helpers (spec §4.4/§10, M3 子项目 B1): a per-doc debouncer for
// shadow writes plus display formatters for the recovery banner. DOM-free so the
// timing and formatting rules are unit-testable.

/** Coalesce shadow writes: one timer per doc id, reset on every change. */
export class Debouncer {
  private readonly delayMs: number;
  private readonly fire: (docId: number) => void;
  private readonly timers = new Map<number, ReturnType<typeof setTimeout>>();

  constructor(delayMs: number, fire: (docId: number) => void) {
    this.delayMs = delayMs;
    this.fire = fire;
  }

  /** (Re)start the timer for `docId`; the pending write fires once it elapses. */
  schedule(docId: number): void {
    this.cancel(docId);
    const t = setTimeout(() => {
      this.timers.delete(docId);
      this.fire(docId);
    }, this.delayMs);
    this.timers.set(docId, t);
  }

  /** Drop a pending write for `docId` (saved/closed/disabled). */
  cancel(docId: number): void {
    const t = this.timers.get(docId);
    if (t !== undefined) {
      clearTimeout(t);
      this.timers.delete(docId);
    }
  }

  cancelAll(): void {
    for (const t of this.timers.values()) clearTimeout(t);
    this.timers.clear();
  }
}

/** Human-readable byte size for the recovery banner (B/KB/MB/GB, ≤1 decimal). */
export function humanSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"] as const;
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  const text = i === 0 ? String(Math.round(v)) : trimTrailingZero(v.toFixed(1));
  return `${text} ${units[i]}`;
}

function trimTrailingZero(s: string): string {
  return s.endsWith(".0") ? s.slice(0, -2) : s;
}

/** Local `YYYY-MM-DD HH:mm:ss` (explicit fields: no locale dependence). */
export function formatSavedAt(ms: number): string {
  const d = new Date(ms);
  const p = (n: number): string => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}
