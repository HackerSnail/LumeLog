// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { clampPage, PdfView, stepZoom } from "./pdfview";

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
