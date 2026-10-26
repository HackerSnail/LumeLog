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
    summary: { events: 0, attachments: 0, groups: [{ index: 0, dataGroup: 0, channels: 3 }] },
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

  it("charts share the selection and apply their window to the table", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const queries: Mf4Query[] = [];
    const series: { channels: number[]; fromSec: number | null; toSec: number | null; buckets: number }[] = [];
    const view = new Mf4View(host, {
      info: { version: "4.10", channelCount: 4, dataGroups: 1, comment: "" },
      summary: null,
      channels: [...channels, { index: 3, name: "Note", unit: "", kind: "text", master: false, group: 0 }],
      hasTimeAxis: true,
      onQuery: (q) => queries.push(q),
      onSeriesQuery: (q) => series.push(q),
      onExport: () => {},
      onClose: () => {},
    });
    host.querySelectorAll<HTMLInputElement>(".mf4-channel input")[0].click();
    // Selecting a text column is allowed in table mode …
    host.querySelectorAll<HTMLInputElement>(".mf4-channel input")[3].click();
    expect(queries.length).toBe(2);

    const tabs = host.querySelectorAll<HTMLButtonElement>(".mf4-tab");
    tabs[1].click();
    expect(series.length).toBe(1);
    // … but the chart must drop it instead of failing the whole request.
    expect(series[0].channels).toEqual([1]);
    expect(series[0].fromSec).toBeNull();
    expect(series[0].toSec).toBeNull(); // null window = resolve full extent

    view.setSeries({
      hasTimeAxis: true,
      fromSec: 0,
      toSec: 10,
      bucketSec: 1,
      times: [0.5, 1.5],
      series: [
        {
          index: 1,
          name: "Speed",
          unit: "",
          degraded: false,
          min: [1, 2],
          max: [1, 2],
          first: [1, 2],
          last: [1, 2],
        },
      ],
    });
    expect(host.querySelector(".mf4-chart-base")).toBeTruthy();
    // Text channels cannot be plotted.
    const boxesAfter = host.querySelectorAll<HTMLInputElement>(".mf4-channel input");
    expect(boxesAfter[3].disabled).toBe(true);

    host.querySelector<HTMLButtonElement>(".mf4-apply-window")!.click();
    const last = queries[queries.length - 1];
    expect(last.fromSec).toBe(0);
    expect(last.toSec).toBe(10);
    host.remove();
  });

  it("disables the waveform tab without a time axis", () => {
    const host = document.createElement("div");
    document.body.append(host);
    new Mf4View(host, {
      info: { version: "4.10", channelCount: 3, dataGroups: 1, comment: "" },
      summary: null,
      channels,
      hasTimeAxis: false,
      onQuery: () => {},
      onSeriesQuery: () => {},
      onExport: () => {},
      onClose: () => {},
    });
    expect(host.querySelectorAll<HTMLButtonElement>(".mf4-tab")[1].disabled).toBe(true);
    host.remove();
  });

  it("does not query while busy and arms export cancellation", () => {
    const host = document.createElement("div");
    document.body.append(host);
    const queries: Mf4Query[] = [];
    let cancelled = 0;
    const view = new Mf4View(host, {
      info: { version: "4.10", channelCount: 3, dataGroups: 1, comment: "" },
      summary: null,
      channels,
      onQuery: (q) => queries.push(q),
      onExport: () => {},
      onCancelExport: () => {
        cancelled += 1;
      },
      onClose: () => {},
    });
    const boxes = host.querySelectorAll<HTMLInputElement>(".mf4-channel input");
    boxes[0].click();
    expect(queries.length).toBe(1);
    view.setBusy(true);
    boxes[1].click();
    expect(queries.length).toBe(1); // busy swallows the change
    view.setBusy(false);
    boxes[1].click();
    expect(queries.length).toBe(2);

    const cancelBtn = host.querySelector<HTMLButtonElement>(".mf4-cancel-export")!;
    expect(cancelBtn.disabled).toBe(true);
    view.setExporting(true);
    expect(cancelBtn.disabled).toBe(false);
    cancelBtn.click();
    expect(cancelled).toBe(1);
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
