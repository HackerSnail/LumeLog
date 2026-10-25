import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Debouncer, formatSavedAt, humanSize } from "./recovery";

describe("Debouncer", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("coalesces repeated schedules into one write", () => {
    const seen: number[] = [];
    const d = new Debouncer(2000, (id) => seen.push(id));
    d.schedule(7);
    d.schedule(7);
    d.schedule(7);
    vi.advanceTimersByTime(1999);
    expect(seen).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(seen).toEqual([7]);
  });

  it("cancel drops the pending write", () => {
    const seen: number[] = [];
    const d = new Debouncer(2000, (id) => seen.push(id));
    d.schedule(1);
    d.cancel(1);
    vi.advanceTimersByTime(5000);
    expect(seen).toEqual([]);
  });

  it("isolates ids and cancelAll clears everything", () => {
    const seen: number[] = [];
    const d = new Debouncer(100, (id) => seen.push(id));
    d.schedule(1);
    d.schedule(2);
    vi.advanceTimersByTime(100);
    expect(seen.sort()).toEqual([1, 2]);

    d.schedule(3);
    d.cancelAll();
    vi.advanceTimersByTime(1000);
    expect(seen.sort()).toEqual([1, 2]);
  });
});

describe("humanSize", () => {
  it("formats byte sizes with <=1 decimal", () => {
    expect(humanSize(0)).toBe("0 B");
    expect(humanSize(1023)).toBe("1023 B");
    expect(humanSize(1024)).toBe("1 KB");
    expect(humanSize(1536)).toBe("1.5 KB");
    expect(humanSize(1.5 * 1024 * 1024)).toBe("1.5 MB");
    expect(humanSize(Number.NaN)).toBe("0 B");
  });
});

describe("formatSavedAt", () => {
  it("renders a stable YYYY-MM-DD HH:mm:ss shape", () => {
    expect(formatSavedAt(1_700_000_000_000)).toMatch(
      /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/,
    );
  });
});
