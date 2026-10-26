// MF4 dock view (Roadmap-4): metadata line, channel search/selection, time window,
// paging, page-local column sort and CSV export. Pure DOM with injected callbacks;
// `main.ts` owns IPC, the file dialog and the session map.

import type { Mf4Channel, Mf4Rows, Mf4Series } from "./ipc";
import { Mf4Table } from "./mf4-table";
import { Mf4Chart, type Win } from "./mf4-chart";

export interface Mf4Query {
  channels: number[];
  fromSec: number | null;
  toSec: number | null;
  sort: string;
  limit: number;
  offset: number;
}

export interface Mf4UiOpts {
  info: { version: string; channelCount: number; dataGroups: number; comment: string };
  summary: {
    events: number;
    attachments: number;
    groups: { index: number; dataGroup: number; channels: number }[];
  } | null;
  channels: Mf4Channel[];
  /** False disables the waveform tab (no time master in the file). */
  hasTimeAxis?: boolean;
  onQuery: (q: Mf4Query) => void;
  onSeriesQuery?: (q: {
    channels: number[];
    fromSec: number | null;
    toSec: number | null;
    buckets: number;
  }) => void;
  onExport: (a: { channels: number[]; fromSec: number | null; toSec: number | null }) => void;
  onCancelExport?: () => void;
  onClose: () => void;
}

const MAX_COLS = 32;
/** Rows per page choices offered by the toolbar. */
const PAGE_SIZES = [100, 500, 1000];

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text = "",
): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  n.textContent = text;
  return n;
}

function numOrNull(v: string): number | null {
  const t = v.trim();
  if (t === "") return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
}

export class Mf4View {
  private readonly opts: Mf4UiOpts;
  private readonly table: Mf4Table;
  private readonly listEl: HTMLElement;
  private readonly statusEl: HTMLElement;
  private readonly fromInput: HTMLInputElement;
  private readonly toInput: HTMLInputElement;
  private readonly sortSel: HTMLSelectElement;
  private readonly sizeSel: HTMLSelectElement;
  private readonly prevBtn: HTMLButtonElement;
  private readonly nextBtn: HTMLButtonElement;
  private readonly pageLabel: HTMLElement;
  private readonly exportBtn: HTMLButtonElement;
  private readonly cancelExportBtn: HTMLButtonElement;
  private readonly searchInput: HTMLInputElement;
  private readonly selected = new Set<number>();
  private limit = 500;
  private offset = 0;
  private sort = "timeAsc";
  private total = 0;
  private busy = false;
  private lastEmpty = "尚未选择通道";
  private lastSelected = -1;
  private mode: "table" | "chart" = "table";
  private readonly tabTable: HTMLButtonElement;
  private readonly tabChart: HTMLButtonElement;
  private readonly chartWrap: HTMLElement;
  private readonly chart: Mf4Chart;
  private readonly chartWindowEl: HTMLElement;
  private chartWindow: Win | null = null;
  private fullWin: Win | null = null;
  private readonly tableWrap: HTMLElement;

  constructor(host: HTMLElement, opts: Mf4UiOpts) {
    this.opts = opts;
    host.textContent = "";
    host.classList.add("mf4-view");

    const head = el("div", "mf4-head");
    const title = el("div", "mf4-title", "测量数据");
    title.title = opts.info.comment;
    const close = el("button", "mf4-close", "关闭");
    close.addEventListener("click", () => opts.onClose());
    head.append(title, close);

    const meta = el(
      "div",
      "mf4-meta",
      `MDF ${opts.info.version} · ${opts.info.channelCount} 通道 · ${opts.info.dataGroups} 数据组` +
        (opts.summary ? ` · ${opts.summary.events} 事件 · ${opts.summary.attachments} 附件` : ""),
    );

    const bar = el("div", "mf4-toolbar");
    this.searchInput = document.createElement("input");
    this.searchInput.className = "mf4-search";
    this.searchInput.placeholder = "搜索通道…";
    let searchTimer: number | null = null;
    this.searchInput.addEventListener("input", () => {
      if (searchTimer !== null) window.clearTimeout(searchTimer);
      searchTimer = window.setTimeout(() => this.renderChannels(), 150);
    });
    bar.append(document.createTextNode("通道"), this.searchInput);

    bar.append(document.createTextNode("时间(s)"));
    this.fromInput = document.createElement("input");
    this.fromInput.className = "mf4-time-input";
    this.fromInput.placeholder = "起";
    this.toInput = document.createElement("input");
    this.toInput.className = "mf4-time-input";
    this.toInput.placeholder = "止";
    const applyWindow = (): void => {
      const from = numOrNull(this.fromInput.value);
      const to = numOrNull(this.toInput.value);
      if (from !== null && to !== null && from > to) {
        this.setStatus("时间窗起点不能晚于止点");
        return;
      }
      this.offset = 0;
      this.query();
    };
    this.fromInput.addEventListener("change", applyWindow);
    this.toInput.addEventListener("change", applyWindow);
    bar.append(this.fromInput, document.createTextNode("–"), this.toInput);

    this.sortSel = document.createElement("select");
    for (const [v, t] of [
      ["timeAsc", "时间升序"],
      ["timeDesc", "时间降序"],
    ]) {
      const o = document.createElement("option");
      o.value = v;
      o.textContent = t;
      this.sortSel.append(o);
    }
    this.sortSel.addEventListener("change", () => {
      this.sort = this.sortSel.value;
      this.offset = 0;
      this.query();
    });
    bar.append(this.sortSel);

    this.sizeSel = document.createElement("select");
    for (const n of PAGE_SIZES) {
      const o = document.createElement("option");
      o.value = String(n);
      o.textContent = `${n}/页`;
      if (n === this.limit) o.selected = true;
      this.sizeSel.append(o);
    }
    this.sizeSel.addEventListener("change", () => {
      this.limit = Number(this.sizeSel.value);
      this.offset = 0;
      this.query();
    });
    bar.append(this.sizeSel);

    this.prevBtn = el("button", "mf4-prev", "上一页");
    this.prevBtn.addEventListener("click", () => {
      this.offset = Math.max(0, this.offset - this.limit);
      this.query();
    });
    this.nextBtn = el("button", "mf4-next", "下一页");
    this.nextBtn.addEventListener("click", () => {
      this.offset += this.limit;
      this.query();
    });
    this.pageLabel = el("span", "mf4-page", "0 行");
    bar.append(this.prevBtn, this.nextBtn, this.pageLabel);

    this.exportBtn = el("button", "mf4-export", "导出 CSV…");
    this.exportBtn.addEventListener("click", () => {
      if (this.selected.size === 0 || this.busy) return;
      this.opts.onExport({
        channels: [...this.selected].sort((a, b) => a - b),
        fromSec: numOrNull(this.fromInput.value),
        toSec: numOrNull(this.toInput.value),
      });
    });
    bar.append(this.exportBtn);
    this.cancelExportBtn = el("button", "mf4-cancel-export", "取消导出");
    this.cancelExportBtn.disabled = true;
    this.cancelExportBtn.addEventListener("click", () => {
      if (this.cancelExportBtn.disabled) return;
      this.cancelExportBtn.disabled = true;
      this.opts.onCancelExport?.();
    });
    bar.append(this.cancelExportBtn);

    this.statusEl = el("span", "mf4-status", "");

    const body = el("div", "mf4-body");
    this.listEl = el("div", "mf4-channels");
    const tableHost = el("div", "mf4-table");
    this.table = new Mf4Table(tableHost);
    body.append(this.listEl, tableHost);
    this.tableWrap = body;

    host.append(head, meta, bar, this.statusEl, body);

    // Table / waveform tabs. The chart shares the channel selection and only
    // re-queries when the tab is active or its own window changes.
    const tabs = el("div", "mf4-tabs");
    this.tabTable = el("button", "mf4-tab active", "表格");
    this.tabChart = el("button", "mf4-tab", "波形");
    this.tabChart.disabled = opts.hasTimeAxis === false;
    this.tabTable.addEventListener("click", () => this.setMode("table"));
    this.tabChart.addEventListener("click", () => this.setMode("chart"));
    tabs.append(this.tabTable, this.tabChart);

    this.chartWrap = el("div", "mf4-chart-wrap");
    this.chartWrap.style.display = "none";
    const chartBar = el("div", "mf4-toolbar");
    this.chartWindowEl = el("span", "mf4-page", "");
    const applyBtn = el("button", "mf4-apply-window", "应用到表格");
    applyBtn.addEventListener("click", () => this.applyChartWindow());
    const resetBtn = el("button", "mf4-reset-window", "复位");
    resetBtn.addEventListener("click", () => {
      if (!this.fullWin) return;
      this.chart.setWindow({ ...this.fullWin });
      this.chartWindow = this.chart.window;
      this.syncChartWindowLabel();
      this.emitSeries();
    });
    chartBar.append(this.chartWindowEl, applyBtn, resetBtn);
    const chartHost = el("div", "mf4-chart-host");
    this.chart = new Mf4Chart(chartHost);
    this.chart.onWindowChange((w) => {
      this.chartWindow = w;
      this.syncChartWindowLabel();
      this.emitSeries();
    });
    this.chartWrap.append(chartBar, chartHost);
    host.append(tabs, this.chartWrap);
    host.insertBefore(tabs, bar); // tabs stay above the toolbar in both modes
    this.renderChannels();
    this.syncButtons();
  }

  /** Feed one backend page. Also records the window total for paging. */
  setRows(rows: Mf4Rows): void {
    this.total = rows.totalRows;
    if (rows.columns.length === 0) {
      this.table.setEmpty(this.lastEmpty);
    } else {
      this.table.setData(rows);
    }
    const first = rows.timeMs.length === 0 ? 0 : this.offset + 1;
    const last = this.offset + rows.timeMs.length;
    this.pageLabel.textContent =
      rows.timeMs.length === 0
        ? `无样本 · 共 ${rows.totalRows} 行`
        : `${first}–${last} / 共 ${rows.totalRows} 行` +
          (rows.hasTimeAxis ? "" : " · 无时间轴");
    this.syncButtons();
  }

  setStatus(text: string): void {
    this.statusEl.textContent = text;
  }

  setBusy(busy: boolean): void {
    this.busy = busy;
    this.chart.setBusy(busy);
    this.syncButtons();
  }

  /** Export runs in the backend with its own cancel hook; only then is it armed. */
  setExporting(exporting: boolean): void {
    this.cancelExportBtn.disabled = !exporting;
  }

  /** Feed downsampled series. The file's full extent is fixed by the first
   *  (windowless) response; later responses never move the user's window. */
  setSeries(data: Mf4Series): void {
    if (!this.fullWin) {
      this.fullWin = {
        start: data.fromSec,
        end: data.toSec > data.fromSec ? data.toSec : data.fromSec + 1,
      };
      this.chart.setFullRange(this.fullWin);
      this.chartWindow = { ...this.fullWin };
    }
    this.chart.setData(data);
    this.chart.setWindow(this.chartWindow ?? this.fullWin);
    this.syncChartWindowLabel();
  }

  /** Detach chart listeners before the dock is cleared. */
  destroy(): void {
    this.chart.destroy();
  }

  private setMode(mode: "table" | "chart"): void {
    if (mode === "chart" && this.opts.hasTimeAxis === false) return;
    this.mode = mode;
    if (mode === "chart") {
      // Text/unknown channels and the time master itself cannot be plotted; drop
      // them from the selection so the checkbox list and the chart agree.
      for (const idx of [...this.selected]) {
        const ch = this.opts.channels.find((c) => c.index === idx);
        if (!ch || ch.kind !== "number" || ch.master) this.selected.delete(idx);
      }
      this.lastSelected = -1;
    }
    this.tabTable.classList.toggle("active", mode === "table");
    this.tabChart.classList.toggle("active", mode === "chart");
    this.tableWrap.style.display = mode === "table" ? "" : "none";
    this.chartWrap.style.display = mode === "chart" ? "" : "none";
    this.renderChannels();
    if (mode === "chart") this.emitSeries();
  }

  private syncChartWindowLabel(): void {
    const w = this.chartWindow;
    this.chartWindowEl.textContent = w
      ? `窗口 ${w.start.toFixed(3)} – ${w.end.toFixed(3)} s`
      : "";
  }

  /** Ask main for the envelope of the current selection over the chart window. */
  private emitSeries(): void {
    if (this.mode !== "chart" || !this.opts.onSeriesQuery) return;
    const numeric = [...this.selected].filter((idx) => {
      const ch = this.opts.channels.find((c) => c.index === idx);
      return ch?.kind === "number" && !ch.master;
    });
    if (numeric.length === 0) {
      this.setStatus("请先勾选数值通道");
      return;
    }
    if (numeric.length > 8) {
      this.setStatus("波形最多显示 8 条序列，请先减少选择");
      return;
    }
    const win = this.chartWindow ?? this.fullWin;
    this.opts.onSeriesQuery({
      channels: numeric.sort((a, b) => a - b),
      // Before the first response the chart has no window: ask the backend to
      // resolve that record stream's extent (null window).
      fromSec: win ? win.start : numOrNull(this.fromInput.value),
      toSec: win ? win.end : numOrNull(this.toInput.value),
      buckets: 2000,
    });
  }

  /** Copy the chart window into the table inputs and reload that page. */
  private applyChartWindow(): void {
    const win = this.chartWindow;
    if (!win) return;
    this.fromInput.value = win.start.toFixed(6);
    this.toInput.value = win.end.toFixed(6);
    this.offset = 0;
    this.query();
  }

  private syncButtons(): void {
    this.prevBtn.disabled = this.busy || this.offset === 0;
    this.nextBtn.disabled = this.busy || this.offset + this.limit >= this.total;
    this.exportBtn.disabled = this.busy || this.selected.size === 0;
    this.exportBtn.textContent = `导出 CSV（${this.selected.size} 列）…`;
  }

  private query(): void {
    if (this.busy) return; // one request at a time; stale ones are dropped anyway
    if (this.selected.size === 0) {
      this.lastEmpty = "请先在左侧勾选通道";
      this.table.setEmpty(this.lastEmpty);
      this.pageLabel.textContent = "0 行";
      return;
    }
    this.opts.onQuery({
      channels: [...this.selected].sort((a, b) => a - b),
      fromSec: numOrNull(this.fromInput.value),
      toSec: numOrNull(this.toInput.value),
      sort: this.sort,
      limit: this.limit,
      offset: this.offset,
    });
  }

  private renderChannels(): void {
    const needle = this.searchInput.value.trim().toLowerCase();
    this.listEl.textContent = "";
    const shown = this.opts.channels.filter(
      (c) => needle === "" || c.name.toLowerCase().includes(needle),
    );
    const isChart = this.mode === "chart";
    const cap = isChart ? 8 : MAX_COLS;
    for (const ch of shown) {
      const row = el("label", "mf4-channel");
      const box = document.createElement("input");
      box.type = "checkbox";
      box.checked = this.selected.has(ch.index);
      // Waveforms only make sense for numeric channels, and eight is the cap.
      box.disabled = isChart && ch.kind !== "number";
      box.addEventListener("change", () => {
        if (box.checked) {
          if (this.selected.size >= cap) {
            box.checked = false;
            this.setStatus(`最多选择 ${cap} 个通道`);
            return;
          }
          this.selected.add(ch.index);
        } else {
          this.selected.delete(ch.index);
        }
        this.offset = 0;
        this.syncButtons();
        if (this.mode === "chart") this.emitSeries();
        else this.query();
      });
      const label = el(
        "span",
        "mf4-channel-name",
        ch.master ? `${ch.name} · 时间` : ch.unit ? `${ch.name} (${ch.unit})` : ch.name,
      );
      row.append(box, label);
      this.listEl.append(row);
    }
    if (shown.length === 0) this.listEl.append(el("div", "mf4-empty", "无匹配通道"));
    if (this.selected.size !== this.lastSelected) {
      this.lastSelected = this.selected.size;
      this.setStatus(`已选 ${this.selected.size}/${cap}`);
    }
  }
}
