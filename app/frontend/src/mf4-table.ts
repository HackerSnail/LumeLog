// MF4 record table (Roadmap-4). Renders one backend page (≤1000 rows) as a plain
// table; column sorting is page-local by design (the backend only sorts by time
// across pages). No virtualization: a page is bounded, and keeping the DOM flat
// makes copy/paste and jsdom tests behave like the real thing.

import type { Mf4Cell, Mf4Column, Mf4Rows } from "./ipc";

export function cellText(c: Mf4Cell): string {
  if (c === null || c === undefined) return "—";
  if (typeof c === "boolean") return c ? "✓" : "";
  if (typeof c === "number") {
    if (!Number.isFinite(c)) return "—";
    return Number.isInteger(c) ? String(c) : String(Number(c.toFixed(6)));
  }
  return c;
}

export class Mf4Table {
  private readonly root: HTMLElement;
  private rows: { timeMs: (number | null)[]; columns: Mf4Column[] } | null = null;
  private order: number[] = [];
  private sortKey: { col: number; desc: boolean } | null = null;
  private emptyText = "尚未选择通道";

  constructor(root: HTMLElement) {
    this.root = root;
  }

  setData(data: Mf4Rows): void {
    this.rows = {
      timeMs: data.timeMs,
      columns: data.columns.map((c) => ({ ...c })),
    };
    this.sortKey = null;
    this.order = data.timeMs.map((_, i) => i);
    this.render();
  }

  setEmpty(text: string): void {
    this.emptyText = text;
    this.rows = null;
    this.render();
  }

  /** Page-local sort (never paginates differently); toggles asc/desc. */
  sortByColumn(col: number): void {
    if (!this.rows) return;
    this.sortKey =
      this.sortKey && this.sortKey.col === col
        ? { col, desc: !this.sortKey.desc }
        : { col, desc: false };
    const values = this.rows.columns[col]?.values ?? [];
    const dir = this.sortKey.desc ? -1 : 1;
    this.order = this.order.slice().sort((a, b) => {
      const va = values[a];
      const vb = values[b];
      if (va === null || va === undefined) return 1;
      if (vb === null || vb === undefined) return -1;
      if (typeof va === "number" && typeof vb === "number") return (va - vb) * dir;
      return String(va).localeCompare(String(vb)) * dir;
    });
    this.render();
  }

  private render(): void {
    this.root.textContent = "";
    const table = document.createElement("table");
    table.className = "mf4-table-el";
    const thead = document.createElement("thead");
    const hrow = document.createElement("tr");
    const timeTh = document.createElement("th");
    timeTh.textContent = "时间(ms)";
    hrow.append(timeTh);
    this.rows?.columns.forEach((c, i) => {
      const th = document.createElement("th");
      th.textContent = c.unit ? `${c.name} (${c.unit})` : c.name;
      if (c.degraded) {
        th.textContent = `⚠ ${th.textContent}`;
        th.title = "该通道无法读取为单元格（类型不支持或读取失败）";
        th.classList.add("degraded");
      }
      if (this.sortKey?.col === i) {
        th.textContent += this.sortKey.desc ? " ▾" : " ▴";
      }
      th.addEventListener("click", () => this.sortByColumn(i));
      hrow.append(th);
    });
    thead.append(hrow);
    table.append(thead);
    const tbody = document.createElement("tbody");
    if (!this.rows) {
      const tr = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = 1;
      td.className = "mf4-empty";
      td.textContent = this.emptyText;
      tr.append(td);
      tbody.append(tr);
    } else {
      for (const rowIdx of this.order) {
        const tr = document.createElement("tr");
        const tdTime = document.createElement("td");
        const t = this.rows.timeMs[rowIdx];
        tdTime.textContent = t === null || t === undefined ? "—" : t.toFixed(3);
        tdTime.className = "mf4-time";
        tr.append(tdTime);
        for (const col of this.rows.columns) {
          const td = document.createElement("td");
          const cell = col.values[rowIdx];
          td.textContent = cellText(cell);
          if (cell === null || cell === undefined) td.classList.add("mf4-null");
          tr.append(td);
        }
        tbody.append(tr);
      }
    }
    table.append(tbody);
    this.root.append(table);
  }
}
