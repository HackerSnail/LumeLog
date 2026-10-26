// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import { Mf4Chart, type Win } from "./mf4-chart";
import type { Mf4Series } from "./ipc";

const data = (): Mf4Series => ({
  hasTimeAxis: true,
  fromSec: 0,
  toSec: 10,
  bucketSec: 0.5,
  times: [0.25, 0.75, 1.25, 1.75],
  series: [
    {
      index: 1,
      name: "Speed",
      unit: "km/h",
      degraded: false,
      min: [10, 20, 30, 40],
      max: [12, 22, 32, 42],
      first: [10, 20, 30, 40],
      last: [12, 22, 32, 42],
    },
  ],
});

function mount(): { host: HTMLElement; chart: Mf4Chart } {
  const host = document.createElement("div");
  document.body.append(host);
  return { host, chart: new Mf4Chart(host) };
}

const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

afterEach(() => {
  document.body.textContent = "";
});

describe("Mf4Chart", () => {
  it("mounts two layers and sizes the canvas", async () => {
    const { host, chart } = mount();
    expect(host.querySelectorAll("canvas").length).toBe(2);
    chart.setData(data());
    await wait(20);
    const base = host.querySelector<HTMLCanvasElement>(".mf4-chart-base")!;
    expect(base.width).toBeGreaterThan(0);
    expect(base.height).toBeGreaterThan(0);
    expect(host.querySelector(".mf4-chart-tip")).toBeTruthy();
    chart.destroy();
  });

  it("throttles window changes and resets on double click", async () => {
    const { host, chart } = mount();
    const seen: Win[] = [];
    chart.onWindowChange((w) => seen.push(w));
    chart.setData(data());
    chart.setWindow({ start: 2, end: 4 });
    const overlay = host.querySelector<HTMLCanvasElement>(".mf4-chart-overlay")!;
    overlay.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, clientX: 10, cancelable: true }));
    overlay.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, clientX: 10, cancelable: true }));
    await wait(90);
    expect(seen.length).toBe(1); // two wheels inside one 60ms window
    const zoomed = seen[0];
    expect(zoomed.end - zoomed.start).toBeCloseTo(2 / 1.25 / 1.25, 6);

    overlay.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await wait(90);
    expect(seen[seen.length - 1]).toEqual({ start: 0, end: 10 });
    chart.destroy();
  });

  it("keeps the full extent when a zoom response narrows the data", async () => {
    const { host, chart } = mount();
    const seen: Win[] = [];
    chart.onWindowChange((w) => seen.push(w));
    chart.setFullRange({ start: 0, end: 100 });
    chart.setData(data());
    chart.setWindow({ start: 40, end: 60 });
    // A response covering only the requested window must not redefine `full`.
    chart.setData({ ...data(), fromSec: 40, toSec: 60 });
    const overlay = host.querySelector<HTMLCanvasElement>(".mf4-chart-overlay")!;
    overlay.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    await wait(90);
    expect(seen[seen.length - 1]).toEqual({ start: 0, end: 100 });
    chart.destroy();
  });

  it("renders a legend that hides a series on click", async () => {
    const { host, chart } = mount();
    chart.setData(data());
    await wait(20);
    const item = host.querySelector<HTMLButtonElement>(".mf4-chart-legend-item")!;
    expect(item.textContent).toContain("Speed");
    item.click();
    expect(item.classList.contains("hidden")).toBe(true);
    item.click();
    expect(item.classList.contains("hidden")).toBe(false);
    chart.destroy();
  });

  it("stops reacting after destroy", async () => {
    const { host, chart } = mount();
    const cb = vi.fn();
    chart.onWindowChange(cb);
    chart.setData(data());
    const overlay = host.querySelector<HTMLCanvasElement>(".mf4-chart-overlay")!;
    chart.destroy();
    overlay.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, clientX: 10, cancelable: true }));
    await wait(90);
    expect(cb).not.toHaveBeenCalled();
    expect(host.querySelectorAll("canvas").length).toBe(0);
  });
});
