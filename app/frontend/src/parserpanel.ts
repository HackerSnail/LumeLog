// ParserPanel: generic table for parser-plugin rows (plugin interface,
// increment 2). Trimmed from CanPanel: dynamic columns (from the plugin's
// declared `columns`), no filter/expand/format toggles. Rows stream in through
// a `RowStore` the caller owns; only the visible ± buffer window paints, the
// same strategy as listview/canpanel. Row click reports the row's 1-based
// source line for the editor jump. CSS reuses the `can-*` classes so both
// tables stay visually identical.

import { createTable, getCoreRowModel, type ColumnDef, type Table, type TableState } from "@tanstack/table-core";
import type { ParserRow } from "./ipc";
import { visibleWindow, ROW_HEIGHT } from "./listview";

export interface ParserColumnSpec {
  key: string;
  label: string;
}

/** Accumulates parsed rows in arrival order; the table's data source. */
export class RowStore {
  private rows: ParserRow[] = [];
  private rev = 0;

  push(rows: ParserRow[]): void {
    for (const r of rows) this.rows.push(r);
    this.rev++;
  }

  all(): readonly ParserRow[] {
    return this.rows;
  }

  clear(): void {
    this.rows = [];
    this.rev++;
  }

  get length(): number {
    return this.rows.length;
  }

  get revision(): number {
    return this.rev;
  }
}

export interface ParserPanelOpts {
  /** Fires with the row's 1-based source line when a row is activated. */
  onRowActivate(lineNumber: number): void;
  /** Surfaces a render-time problem in Chinese (never throws into the host). */
  onError(e: string): void;
}

export class ParserPanel {
  private store: RowStore | null = null;
  private columns: ParserColumnSpec[] = [];
  private baseState: TableState | null = null;

  private cachedRev = -1;
  private cachedCols: ParserColumnSpec[] | null = null;
  private cachedTable: Table<ParserRow> | null = null;
  private lastFirst = -1;
  private lastLast = -1;
  private forceRepaint = false;
  private renderQueued = false;

  private root!: HTMLElement;
  private thead!: HTMLElement;
  private bodyScroller!: HTMLElement;
  private spacer!: HTMLElement;
  private rowsBox!: HTMLElement;
  private statusEl!: HTMLElement;

  private scrollScheduled = false;
  private destroyed = false;

  constructor(host: HTMLElement, opts: ParserPanelOpts) {
    this.opts = opts;
    this.buildSkeleton(host);
  }

  private opts: ParserPanelOpts;

  attachStore(store: RowStore): void {
    this.store = store;
    this.forceRepaint = true;
    this.render();
  }

  /** Swap the column set (plugin change / reload); clears painted rows. */
  setColumns(columns: ParserColumnSpec[]): void {
    this.columns = columns;
    this.forceRepaint = true;
    this.render();
  }

  setStatus(text: string): void {
    this.statusEl.textContent = text;
  }

  destroy(): void {
    this.destroyed = true;
    this.root.remove();
  }

  // ---- DOM skeleton ---------------------------------------------------------

  private buildSkeleton(host: HTMLElement): void {
    this.root = document.createElement("div");
    this.root.className = "canpanel";

    const toolbar = document.createElement("div");
    toolbar.className = "can-toolbar";
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
    host.append(this.root);
  }

  // ---- column model (table-core) --------------------------------------------

  private table(rows: readonly ParserRow[]): Table<ParserRow> {
    const columns: ColumnDef<ParserRow>[] = [
      { id: "line", header: "行号", accessorFn: (r) => String(r.line) },
      ...this.columns.map(
        (c, i): ColumnDef<ParserRow> => ({
          id: c.key,
          header: c.label,
          accessorFn: (r) => r.cells[i] ?? "",
        }),
      ),
    ];
    const base = {
      data: rows as ParserRow[],
      columns,
      getCoreRowModel: getCoreRowModel(),
      onStateChange: () => {},
      renderFallbackValue: null,
    };
    if (this.baseState === null) {
      this.baseState = createTable<ParserRow>({ ...base, state: {} as TableState }).initialState;
    }
    return createTable<ParserRow>({ ...base, state: this.baseState });
  }

  // ---- rendering ------------------------------------------------------------

  render(): void {
    if (this.destroyed || this.renderQueued) return;
    this.renderQueued = true;
    requestAnimationFrame(() => {
      this.renderQueued = false;
      this.paintNow();
    });
  }

  private paintNow(): void {
    if (this.destroyed) return;
    try {
      this.renderInner();
    } catch (e) {
      this.opts.onError(`解析表格渲染出错：${String(e)}`);
    }
  }

  private renderInner(): void {
    const store = this.store;
    const rev = store ? store.revision : 0;
    const dataChanged =
      rev !== this.cachedRev || this.columns !== this.cachedCols;
    if (dataChanged) {
      this.cachedTable = this.table(store ? store.all() : []);
      this.renderHeader(this.cachedTable);
      this.cachedRev = rev;
      this.cachedCols = this.columns;
    }
    const n = store ? store.length : 0;
    this.spacer.style.height = `${Math.max(n, 1) * ROW_HEIGHT}px`;

    const { first, last } = visibleWindow(
      this.bodyScroller.scrollTop,
      this.bodyScroller.clientHeight,
      n,
    );
    if (dataChanged || this.forceRepaint || first !== this.lastFirst || last !== this.lastLast) {
      this.paintRows(store ? store.all() : [], first, last);
      this.lastFirst = first;
      this.lastLast = last;
      this.forceRepaint = false;
    }
  }

  private renderHeader(table: Table<ParserRow>): void {
    this.thead.textContent = "";
    const hrow = document.createElement("div");
    hrow.className = "can-hrow";
    for (const header of table.getFlatHeaders()) {
      const cell = document.createElement("div");
      cell.className = "can-cell";
      const def = header.column.columnDef.header;
      cell.textContent = typeof def === "string" ? def : header.column.id;
      hrow.append(cell);
    }
    this.thead.append(hrow);
  }

  private paintRows(rows: readonly ParserRow[], first: number, last: number): void {
    this.rowsBox.textContent = "";
    const frag = document.createDocumentFragment();
    for (let i = first; i < last; i++) {
      const r = rows[i];
      if (!r) continue;
      const row = document.createElement("div");
      row.className = "can-row";
      row.dataset.rowIndex = String(i);
      for (const value of [String(r.line), ...r.cells]) {
        const cell = document.createElement("div");
        cell.className = "can-cell";
        cell.textContent = value === "" ? "–" : value;
        row.append(cell);
      }
      row.addEventListener("click", () => this.opts.onRowActivate(r.line));
      frag.append(row);
    }
    this.rowsBox.append(frag);
  }

  // ---- scroll plumbing (mirrors canpanel/listview) ---------------------------

  private onScroll(): void {
    if (this.scrollScheduled) return;
    this.scrollScheduled = true;
    requestAnimationFrame(() => {
      this.scrollScheduled = false;
      this.paintNow();
    });
  }
}
