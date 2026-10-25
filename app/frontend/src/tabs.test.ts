import { describe, expect, it } from "vitest";
import { externalDot } from "./tabs";

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
