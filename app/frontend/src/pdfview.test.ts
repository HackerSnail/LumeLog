// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { clampPage, hitRect, matchItem, PdfView, stepZoom, thumbScale } from "./pdfview";

describe("clampPage / stepZoom", () => {
  it("clamps pages into [1, total]", () => {
    expect(clampPage(1, 10)).toBe(1);
    expect(clampPage(5.7, 10)).toBe(5);
    expect(clampPage(0, 10)).toBe(1);
    expect(clampPage(-3, 10)).toBe(1);
    expect(clampPage(99, 10)).toBe(10);
    expect(clampPage(1, 0)).toBe(1);
    expect(clampPage(NaN, 10)).toBe(1);
  });

  it("steps zoom by 1.25 within [0.5, 4]", () => {
    expect(stepZoom(1, "in")).toBe(1.25);
    expect(stepZoom(1.25, "out")).toBe(1);
    expect(stepZoom(4, "in")).toBe(4);
    expect(stepZoom(0.5, "out")).toBe(0.5);
    expect(stepZoom(3.2, "in")).toBe(4);
  });

  it("computes thumbnail scales within bounds", () => {
    expect(thumbScale(595, 88)).toBeCloseTo(88 / 595);
    expect(thumbScale(0, 88)).toBe(1); // degenerate page → unit scale
    expect(thumbScale(595, 4000)).toBe(4); // clamped high
    expect(thumbScale(59500, 88)).toBeCloseTo(0.05); // clamped low
  });
});

describe("PDF text search math (Plan 24)", () => {
  it("matches case-insensitively, including overlaps", () => {
    expect(matchItem("Hello hello HELLO", "hello")).toEqual([0, 6, 12]);
    expect(matchItem("abcabc", "abc")).toEqual([0, 3]);
    expect(matchItem("abc", "x")).toEqual([]);
    expect(matchItem("abc", "")).toEqual([]);
  });

  it("computes proportional match rects in CSS pixels", () => {
    // identity-ish viewport transform scaled ×2 (dpr 2 → CSS px = device/2)
    const vt = [2, 0, 0, -2, 0, 100];
    const item = { str: "abcdef", transform: [1, 0, 0, 1, 10, 50], width: 60 };
    // fontHeight = 2 (flip×2); itemWidth device = 60*2 = 120 → charW = 20
    const r = hitRect(item, vt, 2, 2, 3)!;
    expect(r).not.toBeNull();
    // left = (item.x 10*2 + char 2*20) / 2 = 30
    expect(r.left).toBeCloseTo(30);
    expect(r.width).toBeCloseTo(30);
    expect(r.height).toBeCloseTo(1);
    // top: tx[5] = 0 (y-flip cancels offset), top = (0 - 2) / 2 = -1
    expect(r.top).toBeCloseTo(-1);
    expect(hitRect({ ...item, str: "" }, vt, 2, 0, 1)).toBeNull();
    expect(hitRect(item, [1, 2, 3], 1, 0, 1)).toBeNull(); // bad transform
  });
});

describe("PdfView guards", () => {
  it("render before load is a no-op and destroy stops everything", async () => {
    const host = document.createElement("div");
    document.body.append(host);
    const errors: string[] = [];
    const panel = new PdfView(host, { onError: (e) => errors.push(e) });

    await panel.render(1, 1); // no document yet → no throw
    expect(host.querySelectorAll("canvas")).toHaveLength(1);

    panel.destroy();
    expect(host.querySelectorAll("canvas")).toHaveLength(0);
    await panel.render(1, 1); // destroyed → no throw
    expect(errors).toEqual([]);
  });
});
