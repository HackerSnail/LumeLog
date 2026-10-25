// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { Statusbar } from "./statusbar";

function mountBar(): { root: HTMLElement; chip: HTMLElement } {
  const root = document.createElement("div");
  document.body.append(root);
  new Statusbar(root);
  const chip = root.querySelector(".status-watch") as HTMLElement;
  expect(chip).toBeTruthy();
  return { root, chip };
}

describe("Statusbar.setWatch (spec §4.4)", () => {
  it("hides the chip while watching works", () => {
    const { chip } = mountBar();
    expect(chip.style.display).toBe("none");
    expect(chip.textContent).toBe("");
  });

  it("shows a warn chip with the degradation reason and clears on ok", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const bar = new Statusbar(root);
    const chip = root.querySelector(".status-watch") as HTMLElement;

    bar.setWatch("degraded", "notify 不可用");
    expect(chip.style.display).toBe("");
    expect(chip.textContent).toContain("已降级");
    expect(chip.title).toBe("notify 不可用");

    bar.setWatch("ok");
    expect(chip.style.display).toBe("none");
    expect(chip.textContent).toBe("");
    expect(chip.title).toBe("");
  });
});
