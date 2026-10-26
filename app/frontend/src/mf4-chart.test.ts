import { describe, expect, it } from "vitest";
import {
  bucketIndex,
  clampWindow,
  nearestBucket,
  panBy,
  xOf,
  yOf,
  zoomAt,
  type Win,
} from "./mf4-chart";

const full: Win = { start: 0, end: 100 };

describe("mf4 chart math", () => {
  it("zoom keeps the anchor under the cursor", () => {
    const win: Win = { start: 20, end: 60 };
    const anchor = 35;
    const left = xOf(anchor, win, 0, 800);
    const zoomed = zoomAt(win, 0.5, anchor, full);
    const leftAfter = xOf(anchor, zoomed, 0, 800);
    expect(Math.abs(leftAfter - left)).toBeLessThan(1e-6);
    expect(zoomed.end - zoomed.start).toBeCloseTo(20, 9);
  });

  it("zoom clamps to the full range and to the minimum span", () => {
    const out = zoomAt({ start: 0, end: 10 }, 0.5, 0, full);
    expect(out.start).toBe(0);
    const tiny = zoomAt({ start: 40, end: 60 }, 1e-12, 50, full);
    expect(tiny.end - tiny.start).toBeGreaterThan(0);
    expect(tiny.end - tiny.start).toBeCloseTo(100 / 1e6, 9);
    const wide = zoomAt({ start: 40, end: 60 }, 1e6, 50, full);
    expect(wide).toEqual(full);
  });

  it("pan clamps at both ends and rejects degenerate windows", () => {
    expect(panBy({ start: 10, end: 30 }, -50, full)).toEqual({ start: 0, end: 20 });
    expect(panBy({ start: 80, end: 90 }, 50, full)).toEqual({ start: 90, end: 100 });
    const bad = clampWindow({ start: 5, end: 5 }, full);
    expect(bad.end).toBeGreaterThan(bad.start);
  });

  it("bucket index clamps edges", () => {
    const win: Win = { start: 0, end: 10 };
    expect(bucketIndex(0, win, 10)).toBe(0);
    expect(bucketIndex(5.1, win, 10)).toBe(5);
    expect(bucketIndex(10, win, 10)).toBe(9); // t == end belongs to the last bucket
    expect(bucketIndex(-1, win, 10)).toBe(0);
    expect(bucketIndex(1, { start: 0, end: 0 }, 10)).toBe(0);
  });

  it("nearest bucket skips empty buckets and handles edges", () => {
    const times = [null, 1, null, 3, 4];
    expect(nearestBucket(times, 0.9)).toBe(1);
    expect(nearestBucket(times, 3.6)).toBe(4);
    expect(nearestBucket(times, 100)).toBe(4);
    expect(nearestBucket([null, null], 1)).toBe(-1);
    expect(nearestBucket([], 1)).toBe(-1);
  });

  it("y mapping is monotonic and flat series stay on the midline", () => {
    expect(yOf(0, 0, 10, 0, 100)).toBe(100);
    expect(yOf(10, 0, 10, 0, 100)).toBe(0);
    expect(yOf(5, 0, 10, 0, 100)).toBeCloseTo(50, 9);
    expect(yOf(5, 5, 5, 0, 100)).toBe(50);
    expect(Number.isNaN(yOf(Number.NaN, 0, 1, 0, 100))).toBe(true);
  });
});
