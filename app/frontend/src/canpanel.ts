// CanPanel: the M2 message table (spec §7.4). Two pieces live here:
//   * `FrameStore`  — pure, DOM-free accumulation of the batches `canParse`
//                     streams in (`onBatch` → `push`); unit-tested in
//                     canpanel.test.ts.
//   * `CanPanel`    — the virtualized table. Columns come from
//                     `@tanstack/table-core`'s row/column model; DOM is built
//                     by hand (`document.createElement`, never innerHTML on frame
//                     data) and only the visible ± buffer window is painted, the
//                     same strategy as listview.ts. A row click reports its
//                     1-based `lineNumber` back through `onRowActivate` so the
//                     editor/list can jump there (Task 7 wiring).

import { createTable, getCoreRowModel, type ColumnDef, type Table, type TableState } from "@tanstack/table-core";
import type { FrameJson } from "./ipc";
import { applyCanFilter, type CanFilter } from "./canfilter";
import { visibleWindow, ROW_HEIGHT } from "./listview";

/** Extra px a signal-detail sub-row adds beneath an expanded frame row. */
const EXPAND_H = 120;

/** Accumulates decoded frames in arrival order; the table's data source. */
export class FrameStore {
  private frames: FrameJson[] = [];

  push(frames: FrameJson[]): void {
    // Copy so a caller reusing its batch array can't mutate our history.
    for (const f of frames) this.frames.push(f);
  }

  all(): readonly FrameJson[] {
    return this.frames;
  }

  clear(): void {
    this.frames = [];
  }

  get length(): number {
    return this.frames.length;
  }
}

export interface CanPanelOpts {
  /** Fires with the frame's 1-based source line when a row is activated. */
  onRowActivate(lineNumber: number): void;
  /** Surfaces a render-time problem in Chinese (never throws into the host). */
  onError(e: string): void;
}

/** Minimal table state the columns read; also drives the header toggles. */
interface ViewFlags {
  idHex: boolean; // true → "0x1A0", false → decimal
  relTime: boolean; // true → relative to earliest ts, false → absolute
  micros: boolean; // true → µs, false → ms
}

export class CanPanel {
  private readonly host: HTMLElement;
  private readonly opts: CanPanelOpts;
  private store: FrameStore | null = null;
  private filter: CanFilter = {};
  private flags: ViewFlags = { idHex: true, relTime: false, micros: false };
  private expanded = new Set<number>(); // filtered-index → open signal row
  private viewMinTs: number | null = null; // earliest ts in the current filtered view
  private baseState: TableState | null = null; // controlled table state (feature defaults, built once)

  private root!: HTMLElement;
  private thead!: HTMLElement;
  private bodyScroller!: HTMLElement;
  private spacer!: HTMLElement;
  private rowsBox!: HTMLElement;
  private statusEl!: HTMLElement;

  private scrollScheduled = false;
  private destroyed = false;

  constructor(host: HTMLElement, opts: CanPanelOpts) {
    this.host = host;
    this.opts = opts;
    this.buildSkeleton();
  }

  attachStore(store: FrameStore): void {
    this.store = store;
    this.expanded.clear();
    this.render();
  }

  setFilter(f: CanFilter): void {
    this.filter = f;
    this.expanded.clear();
    this.render();
  }

  destroy(): void {
    this.destroyed = true;
    this.root.remove();
  }

  // ---- DOM skeleton ---------------------------------------------------------

  private buildSkeleton(): void {
    this.root = document.createElement("div");
    this.root.className = "canpanel";

    const toolbar = document.createElement("div");
    toolbar.className = "can-toolbar";
    toolbar.append(
      this.toggleButton("ID: HEX/DEC", () => (this.flags.idHex = !this.flags.idHex), () => this.flags.idHex),
      this.toggleButton("时间: 绝对/相对", () => (this.flags.relTime = !this.flags.relTime), () => !this.flags.relTime),
      this.toggleButton("单位: ms/µs", () => (this.flags.micros = !this.flags.micros), () => !this.flags.micros),
    );

    this.statusEl = document.createElement("span");
    this.statusEl.className = "can-status";
    toolbar.append(this.statusEl);

    const scroller = document.createElement("div");
    scroller.className = "can-scroll";
    scroller.style.position = "relative";
    scroller.style.overflow = "auto";
    this.bodyScroller = scroller;

    this.spacer = document.createElement("div");
    this.spacer.style.position = "relative";
    this.spacer.style.width = "1px";

    this.rowsBox = document.createElement("div");
    this.rowsBox.style.position = "absolute";
    this.rowsBox.style.left = "0";
    this.rowsBox.style.right = "0";
    this.spacer.append(this.rowsBox);
    scroller.append(this.spacer);

    this.thead = document.createElement("div");
    this.thead.className = "can-thead";

    scroller.addEventListener("scroll", () => this.onScroll());

    this.root.append(toolbar, this.thead, scroller);
    this.host.append(this.root);
  }

  /** A header toggle whose label reflects the current value; repaints on click. */
  private toggleButton(label: string, flip: () => void, isActive: () => boolean): HTMLElement {
    const [on, off] = label.split(": ");
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = "can-toggle";
    btn.textContent = `${on}: ${off!.split("/")[isActive() ? 0 : 1]}`;
    btn.addEventListener("click", () => {
      flip();
      btn.textContent = `${on}: ${off!.split("/")[isActive() ? 0 : 1]}`;
      this.render();
    });
    return btn;
  }

  // ---- column model (table-core) --------------------------------------------

  private table(rows: readonly FrameJson[]): Table<FrameJson> {
    const columns: ColumnDef<FrameJson>[] = [
      { id: "line", header: "行号", accessorKey: "lineNumber" },
      { id: "time", header: "时间", accessorKey: "timestampMs" },
      { id: "channel", header: "通道", accessorKey: "channel" },
      { id: "id", header: "ID", accessorKey: "canId" },
      { id: "msg", header: "消息", accessorKey: "msgName" },
      { id: "dlc", header: "DLC", accessorKey: "dlc" },
      { id: "signals", header: "信号", accessorKey: "signals" },
      { id: "data", header: "原始数据", accessorKey: "dataHex" },
    ];
    // table-core has no framework adapter here, so we drive it as a fully
    // controlled table: the feature-default `initialState` is captured once and
    // fed back as `state` (an empty `state` would crash a feature's first
    // `getState()` read). Data changes flow through `rows`; the state (column
    // visibility/ordering, none of which this v1 panel mutates) stays stable.
    const base = {
      data: rows as FrameJson[],
      columns,
      getCoreRowModel: getCoreRowModel(),
      onStateChange: () => {},
      renderFallbackValue: null,
    };
    if (this.baseState === null) {
      this.baseState = createTable<FrameJson>({ ...base, state: {} as TableState }).initialState;
    }
    return createTable<FrameJson>({ ...base, state: this.baseState });
  }

  // ---- rendering ------------------------------------------------------------

  render(): void {
    if (this.destroyed) return;
    try {
      this.renderInner();
    } catch (e) {
      this.opts.onError(`报文表格渲染出错：${String(e)}`);
    }
  }

  private renderInner(): void {
    const all = this.store ? this.store.all() : [];
    const filtered = applyCanFilter(all as FrameJson[], this.filter);
    const table = this.table(filtered);

    // Header row (rebuilt each render so toggles reflect; cheap, one row).
    this.thead.textContent = "";
    const hrow = document.createElement("div");
    hrow.className = "can-row can-hrow";
    for (const header of table.getFlatHeaders()) {
      const cell = document.createElement("div");
      cell.className = "can-cell";
      const def = header.column.columnDef.header;
      cell.textContent = typeof def === "string" ? def : header.column.id;
      hrow.append(cell);
    }
    this.thead.append(hrow);

    const n = filtered.length;
    this.viewMinTs = this.minTimestamp(filtered);
    const expandCount = this.countExpanded(n);
    this.spacer.style.height = `${Math.max(n, 1) * ROW_HEIGHT + expandCount * EXPAND_H}px`;

    const { first, last } = visibleWindow(this.bodyScrollerTop(), this.bodyScrollerHeight(), n);
    this.paintRows(filtered, first, last);

    this.statusEl.textContent = `报文 ${n}/${all.length}`;
  }

  private paintRows(rows: readonly FrameJson[], first: number, last: number): void {
    this.rowsBox.textContent = "";
    const frag = document.createDocumentFragment();
    // Cumulative extra offset from expanded rows above the window.
    let offset = 0;
    for (let i = 0; i < first; i++) if (this.expanded.has(i)) offset += EXPAND_H;

    for (let i = first; i < last; i++) {
      const f = rows[i];
      const row = this.frameRow(f, i);
      row.style.top = `${i * ROW_HEIGHT + offset}px`;
      frag.append(row);
      if (this.expanded.has(i)) {
        const detail = this.signalDetail(f);
        detail.style.top = `${i * ROW_HEIGHT + offset + ROW_HEIGHT}px`;
        frag.append(detail);
        offset += EXPAND_H;
      }
    }
    this.rowsBox.append(frag);
    this.rowsBox.style.transform = "translateY(0)";
  }

  private frameRow(f: FrameJson, index: number): HTMLElement {
    const row = document.createElement("div");
    row.className = "can-row";
    row.dataset.frameIndex = String(index);
    const cells: Array<string | number> = [
      f.lineNumber,
      this.formatTime(f),
      f.channel,
      this.flags.idHex ? f.canIdHex : String(f.canId),
      f.msgName ?? "–",
      f.dlc,
      f.signals.length ? `${f.signals.length} 信号` : "–",
      f.dataHex !== "" ? f.dataHex : "–",
    ];
    for (const value of cells) {
      const cell = document.createElement("div");
      cell.className = "can-cell";
      cell.textContent = String(value);
      row.append(cell);
    }
    row.addEventListener("click", () => {
      this.expanded.has(index) ? this.expanded.delete(index) : this.expanded.add(index);
      this.opts.onRowActivate(f.lineNumber);
      this.render();
    });
    return row;
  }

  /** Expanded signal breakdown (name / raw / physical / enum / unit). */
  private signalDetail(f: FrameJson): HTMLElement {
    const box = document.createElement("div");
    box.className = "can-signals";
    box.style.position = "absolute";
    box.style.height = `${EXPAND_H}px`;
    box.style.overflow = "auto";
    if (f.signals.length === 0) {
      box.textContent = f.errorFrame ? "错误帧，无数据" : f.isRemote ? "远程帧，无数据" : "无信号（未匹配 DBC 或空载荷）";
      return box;
    }
    const table = document.createElement("table");
    const thead = document.createElement("thead");
    const trh = document.createElement("tr");
    for (const h of ["信号", "原始值", "物理值", "枚举", "单位"]) {
      const th = document.createElement("th");
      th.textContent = h;
      trh.append(th);
    }
    thead.append(trh);
    const tbody = document.createElement("tbody");
    for (const s of f.signals) {
      const tr = document.createElement("tr");
      const add = (t: string) => {
        const td = document.createElement("td");
        td.textContent = t;
        tr.append(td);
      };
      add(s.name);
      add(String(s.raw));
      add(String(s.physical));
      add(s.enumText ?? "–");
      add(s.unit || "–");
      tbody.append(tr);
    }
    table.append(thead, tbody);
    box.append(table);
    return box;
  }

  // ---- time/id formatting ---------------------------------------------------

  private minTimestamp(rows: readonly FrameJson[]): number | null {
    let min: number | null = null;
    for (const f of rows) {
      if (f.timestampMs === undefined) continue;
      if (min === null || f.timestampMs < min) min = f.timestampMs;
    }
    return min;
  }

  private formatTime(f: FrameJson): string {
    if (f.timestampMs === undefined) return "–";
    let v = f.timestampMs;
    if (this.flags.relTime && this.viewMinTs !== null) v = v - this.viewMinTs;
    const unit = this.flags.micros ? "µs" : "ms";
    const num = this.flags.micros ? v * 1000 : v;
    return `${num.toFixed(this.flags.micros ? 0 : 3)} ${unit}`;
  }

  // ---- scroll plumbing (mirrors listview.ts) --------------------------------

  private countExpanded(n: number): number {
    let c = 0;
    for (const i of this.expanded) if (i < n) c++;
    return c;
  }

  private bodyScrollerTop(): number {
    return this.bodyScroller.scrollTop;
  }

  private bodyScrollerHeight(): number {
    return this.bodyScroller.clientHeight;
  }

  private onScroll(): void {
    if (this.scrollScheduled) return;
    this.scrollScheduled = true;
    requestAnimationFrame(() => {
      this.scrollScheduled = false;
      if (!this.destroyed) this.render();
    });
  }
}
