// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { cellText, Mf4Table } from "./mf4-table";
import type { Mf4Rows } from "./ipc";

const rows = (): Mf4Rows => ({
  timeMs: [0, 1000, 2000],
  hasTimeAxis: true,
  totalRows: 3,
  columns: [
    { index: 0, name: "Speed", unit: "km/h", degraded: false, values: [30, 10, 20] },
    { index: 1, name: "Note", unit: "", degraded: false, values: ["a", null, "c"] },
    { index: 2, name: "Odd", unit: "", degraded: true, values: [null, null, null] },
  ],
});

function mount(): { host: HTMLElement; table: Mf4Table } {
  const host = document.createElement("div");
  document.body.append(host);
  return { host, table: new Mf4Table(host) };
}

describe("Mf4Table", () => {
  it("formats cells and marks degraded columns", () => {
    expect(cellText(null)).toBe("—");
    expect(cellText(3)).toBe("3");
    expect(cellText(1.23456789)).toBe("1.234568");
    expect(cellText(true)).toBe("✓");
    expect(cellText("x")).toBe("x");

    const { host, table } = mount();
    table.setData(rows());
    const heads = [...host.querySelectorAll("th")].map((h) => h.textContent);
    expect(heads[0]).toBe("时间(ms)");
    expect(heads[1]).toContain("Speed (km/h)");
    expect(heads[3]).toContain("⚠");
    expect(host.querySelectorAll("tbody tr").length).toBe(3);
    expect(host.querySelector(".mf4-null")).toBeTruthy();
    host.remove();
  });

  it("sorts the current page locally and toggles direction", () => {
    const { host, table } = mount();
    table.setData(rows());
    const speedTh = () => host.querySelectorAll("th")[1] as HTMLElement;
    speedTh().click();
    let firsts = [...host.querySelectorAll("tbody tr")].map((r) => r.children[1].textContent);
    expect(firsts).toEqual(["10", "20", "30"]);
    speedTh().click();
    firsts = [...host.querySelectorAll("tbody tr")].map((r) => r.children[1].textContent);
    expect(firsts).toEqual(["30", "20", "10"]);
    host.remove();
  });

  it("renders a placeholder without data", () => {
    const { host, table } = mount();
    table.setEmpty("该区间无样本");
    expect(host.querySelector(".mf4-empty")!.textContent).toContain("无样本");
    host.remove();
  });
});
