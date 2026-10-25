// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { Statusbar } from "./statusbar";

function mountBar(): { root: HTMLElement; chip: HTMLElement; bar: Statusbar } {
  const root = document.createElement("div");
  document.body.append(root);
  const bar = new Statusbar(root);
  const chip = root.querySelector(".status-watch") as HTMLElement;
  expect(chip).toBeTruthy();
  return { root, chip, bar };
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

describe("Statusbar tail follow (Roadmap)", () => {
  it("shows the toggle only for read-only docs and reflects the state", () => {
    const { root, bar } = mountBar();
    const btn = root.querySelector(".status-follow") as HTMLButtonElement;
    expect(btn.style.display).toBe("none");

    bar.setFollowAvailable(true);
    expect(btn.style.display).toBe("");
    bar.setFollow(true);
    expect(btn.classList.contains("active")).toBe(true);
    expect(btn.getAttribute("aria-pressed")).toBe("true");

    bar.setFollowAvailable(false); // leaving a read-only doc clears it
    expect(btn.style.display).toBe("none");
    expect(btn.classList.contains("active")).toBe(false);
  });

  it("reports toggle changes with onFollow", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const seen: boolean[] = [];
    const bar = new Statusbar(root, { onFollow: (on) => seen.push(on) });
    bar.setFollowAvailable(true);
    const btn = root.querySelector(".status-follow") as HTMLButtonElement;
    btn.click();
    btn.click();
    expect(seen).toEqual([true, false]);
  });
});
