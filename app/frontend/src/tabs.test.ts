// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { externalDot, TabStrip, type TabView } from "./tabs";

// The tab strip itself is DOM-only (no jsdom in this project); its yellow-dot
// presentation is factored into this pure helper so the on/off contract is
// pinned headlessly.
describe("externalDot (spec §4.4)", () => {
  it("shows a warn dot with a tooltip when externally changed", () => {
    expect(externalDot(true)).toEqual({
      text: "●",
      warn: true,
      title: "文件已被外部修改",
    });
  });

  it("clears text, warn class and tooltip when resolved", () => {
    expect(externalDot(false)).toEqual({ text: "", warn: false, title: "" });
  });
});

function stubView(docId: number, dirty = false): TabView {
  return {
    docId,
    root: document.createElement("div"),
    isDirty: () => dirty,
    focus: () => {},
    destroy: () => {},
  };
}

describe("TabStrip external flag (real DOM)", () => {
  it("toggles the yellow dot without touching the dirty dot", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const strip = new TabStrip(host);
    strip.add({ docId: 1, path: "C:/logs/a.log" }, stubView(1));
    strip.refreshDirty(1);
    const dirty = host.querySelector(".dirty-dot") as HTMLElement;
    const ext = host.querySelector(".ext-dot") as HTMLElement;

    expect(ext.classList.contains("warn")).toBe(false);
    strip.setExternalFlag(1, true);
    expect(ext.textContent).toBe("●");
    expect(ext.classList.contains("warn")).toBe(true);
    expect(ext.title).toContain("外部修改");

    // A clean doc keeps its dirty dot hidden while externally flagged.
    strip.refreshDirty(1);
    expect(dirty.classList.contains("visible")).toBe(false);

    strip.setExternalFlag(1, false);
    expect(ext.classList.contains("warn")).toBe(false);
    expect(ext.textContent).toBe("");
  });

  it("keeps dirty and external markers independent", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const strip = new TabStrip(host);
    strip.add({ docId: 2, path: "C:/logs/b.log" }, stubView(2, true));
    strip.refreshDirty(2);
    const dirty = host.querySelector(".dirty-dot") as HTMLElement;
    const ext = host.querySelector(".ext-dot") as HTMLElement;
    expect(dirty.classList.contains("visible")).toBe(true);
    expect(ext.classList.contains("warn")).toBe(false);

    strip.setExternalFlag(2, true);
    expect(dirty.classList.contains("visible")).toBe(true);
    expect(ext.classList.contains("warn")).toBe(true);
  });
});
