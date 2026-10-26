// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { Mf4View, type Mf4Query } from "./mf4-ui";
import type { Mf4Rows } from "./ipc";

const channels = [
  { index: 1, name: "Speed", unit: "km/h", kind: "number" as const, master: false, group: 0 },
  { index: 2, name: "EngineTemp", unit: "degC", kind: "number" as const, master: false, group: 0 },
  { index: 0, name: "time", unit: "s", kind: "number" as const, master: true, group: 0 },
];

const page = (n: number, totalRows: number): Mf4Rows => ({
  timeMs: Array.from({ length: n }, (_, i) => i * 1000),
  totalRows,
  hasTimeAxis: true,
  columns: [
    {
      index: 1,
      name: "Speed",
      unit: "km/h",
      degraded: false,
      values: Array.from({ length: n }, (_, i) => i),
    },
  ],
});

function mount(): {
  host: HTMLElement;
  view: Mf4View;
  queries: Mf4Query[];
  exports: { channels: number[]; fromSec: number | null; toSec: number | null }[];
  closed: () => boolean;
} {
  const host = document.createElement("div");
  document.body.append(host);
  const queries: Mf4Query[] = [];
  const exports: { channels: number[]; fromSec: number | null; toSec: number | null }[] = [];
  let close = false;
  const view = new Mf4View(host, {
    info: { version: "4.10", channelCount: 3, dataGroups: 1, comment: "" },
    summary: { events: 0, attachments: 0, groups: [{ index: 0, channels: 3 }] },
    channels,
    onQuery: (q) => queries.push(q),
    onExport: (a) => exports.push(a),
    onClose: () => {
      close = true;
    },
  });
  return { host, view, queries, exports, closed: () => close };
}

const boxes = (host: HTMLElement): HTMLInputElement[] => [
  ...host.querySelectorAll<HTMLInputElement>(".mf4-channel input"),
];

describe("Mf4View", () => {
  it("renders channels and queries on selection", () => {
    const { host, queries } = mount();
    expect(boxes(host).length).toBe(3);
    boxes(host)[0].click();
    expect(queries.length).toBe(1);
    expect(queries[0].channels).toEqual([1]);
    expect(queries[0].offset).toBe(0);
    expect(queries[0].sort).toBe("timeAsc");
    host.remove();
  });

  it("pages with the window total and disables at the edges", () => {
    const { host, view, queries } = mount();
    boxes(host)[0].click();
    view.setRows(page(500, 1200));
    const next = host.querySelector<HTMLButtonElement>(".mf4-next")!;
    const prev = host.querySelector<HTMLButtonElement>(".mf4-prev")!;
    expect(prev.disabled).toBe(true);
    expect(host.querySelector(".mf4-page")!.textContent).toContain("共 1200 行");
    next.click();
    expect(queries[queries.length - 1]!.offset).toBe(500);
    view.setRows(page(500, 1200));
    prev.click();
    expect(queries[queries.length - 1]!.offset).toBe(0);
    host.remove();
  });

  it("exports the selection with the current window only", () => {
    const { host, exports } = mount();
    const exportBtn = host.querySelector<HTMLButtonElement>(".mf4-export")!;
    expect(exportBtn.disabled).toBe(true);
    boxes(host)[1].click();
    expect(exportBtn.disabled).toBe(false);
    exportBtn.click();
    expect(exports[exports.length - 1]).toEqual({ channels: [2], fromSec: null, toSec: null });

    const from = host.querySelectorAll<HTMLInputElement>(".mf4-time-input")[0];
    const to = host.querySelectorAll<HTMLInputElement>(".mf4-time-input")[1];
    from.value = "1.5";
    to.value = "9";
    from.dispatchEvent(new Event("change"));
    exportBtn.click();
    expect(exports[exports.length - 1]).toEqual({ channels: [2], fromSec: 1.5, toSec: 9 });
    host.remove();
  });

  it("rejects an inverted window and closes on demand", () => {
    const { host, queries, closed } = mount();
    boxes(host)[0].click();
    const [from, to] = host.querySelectorAll<HTMLInputElement>(".mf4-time-input");
    from.value = "5";
    to.value = "1";
    const before = queries.length;
    to.dispatchEvent(new Event("change"));
    expect(host.querySelector(".mf4-status")!.textContent).toContain("起点不能晚于止点");
    expect(queries.length).toBe(before);
    host.querySelector<HTMLButtonElement>(".mf4-close")!.click();
    expect(closed()).toBe(true);
    host.remove();
  });
});
