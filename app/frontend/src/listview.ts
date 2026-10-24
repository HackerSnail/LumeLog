//! Read-only virtual list for large files (spec §4.3, >200 MB path). Renders a
//! fixed-row-height window (visible ± `OVERSCAN` lines) and requests exactly one
//! `doc_chunk` per window move — the whole file is never resident. Rows are
//! absolutely positioned against a full-height spacer, so a 1 GiB log scrolls
//! smoothly while only a few thousand line strings exist at a time.
//!
//! This is a *view*, not an editor: `isDirty()` is always false, there is no
//! cursor, and the only interaction is scroll + `scrollToLine`. Encoding switches
//! re-decode by clearing the cache and refetching the live window (`invalidate`).

import type { TabView } from "./tabs";

/** Fixed row height in px — the scroll↔line mapping is arithmetic, not binary. */
export const ROW_HEIGHT = 20;
/** Lines kept above/below the viewport before a refetch (plan: ±200). */
export const OVERSCAN = 200;

export interface ListViewOpts {
  docId: number;
  /** Live total line count — grows as the background index completes (Task 11). */
  totalLines: () => number;
  /** Lines whose byte offsets are indexed so far; `[indexedLines, totalLines)`
   *  renders as a "待索引" placeholder and is never fetched. Equals `totalLines`
   *  while indexing is synchronous (v1), streams ahead of it in Task 11. */
  indexedLines: () => number;
  /** Fetch one window of decoded lines. Injected so the view has no IPC import. */
  loadWindow: (
    start: number,
    count: number,
  ) => Promise<{ first: number; lines: string[]; invalidCount: number }>;
  /** Report decoded-but-unmappable counts for the live window (spec §5.4: the
   *  status bar shows a persistent ⚠ so a wrong encoding is discoverable). */
  onInvalidCount?: (count: number) => void;
}

/**
 * Pure window math (unit-tested in `listview.test.ts`, no DOM). Returns the
 * half-open line range `[first, last)` to render for a given scroll position,
 * clamped to `[0, totalLines)`.
 */
export function visibleWindow(
  scrollTop: number,
  viewportHeight: number,
  totalLines: number,
  rowHeight = ROW_HEIGHT,
  overscan = OVERSCAN,
): { first: number; last: number } {
  const firstRaw = Math.floor(scrollTop / rowHeight) - overscan;
  const lastRaw = Math.floor((scrollTop + viewportHeight) / rowHeight) + 1 + overscan;
  const first = Math.max(0, Math.min(firstRaw, totalLines));
  const last = Math.max(first, Math.min(lastRaw, totalLines));
  return { first, last };
}

export class ListView implements TabView {
  readonly docId: number;
  readonly root: HTMLElement;

  private readonly spacer: HTMLElement;
  private readonly rows: HTMLElement;
  private readonly opts: ListViewOpts;

  /** Cache of `[cacheStart, cacheEnd)` line index → text; replaced per fetch. */
  private cache = new Map<number, string>();
  private cacheStart = -1;
  private cacheEnd = -1;
  /** Last `[first, last)` actually rendered — skip work when it hasn't moved. */
  private renderFirst = -1;
  private renderLast = -1;
  /** Guards against overlapping async fetches racing a fast scroll. */
  private seq = 0;
  private destroyed = false;
  private scrollScheduled = false;

  constructor(opts: ListViewOpts) {
    this.opts = opts;
    this.docId = opts.docId;

    this.root = document.createElement("div");
    this.root.className = "listview";
    this.root.tabIndex = 0; // focusable so keyboard scroll works; read-only otherwise
    this.root.style.position = "relative";
    this.root.style.overflow = "auto";
    // Height is owned by the host layout (`styles.css`: `#tabs .listview` is a
    // bounded flex child). Never set a percentage/auto height here — that is
    // what let the box grow to the full spacer height and request the whole
    // file in one go (final-review C-1).

    this.spacer = document.createElement("div");
    this.spacer.style.position = "relative";
    this.spacer.style.width = "1px"; // width driven by the host; 1px keeps a valid box

    this.rows = document.createElement("div");
    this.rows.style.position = "absolute";
    this.rows.style.left = "0";
    this.rows.style.right = "0";
    this.spacer.append(this.rows);
    this.root.append(this.spacer);

    this.root.addEventListener("scroll", () => this.onScroll());
    this.refresh();
  }

  /** Re-measure total height and (re)render — call after open + on index progress. */
  refresh(): void {
    const total = this.opts.totalLines();
    this.spacer.style.height = `${Math.max(total, 1) * ROW_HEIGHT}px`;
    this.render(true);
  }

  /** Drop the decoded cache and re-render — used by an encoding re-decode. */
  invalidate(): void {
    this.cache.clear();
    this.cacheStart = -1;
    this.cacheEnd = -1;
    this.render(true);
  }

  /** Bring an absolute (0-based) line into view; keeps it near the top. */
  scrollToLine(zeroBasedLine: number): void {
    const total = this.opts.totalLines();
    const line = Math.min(Math.max(0, zeroBasedLine), Math.max(0, total - 1));
    this.root.scrollTop = line * ROW_HEIGHT;
    this.render(true);
  }

  // ---- TabView surface (read-only: never dirty, cheap focus/destroy) ----------

  isDirty(): boolean {
    return false;
  }

  focus(): void {
    this.root.focus();
  }

  destroy(): void {
    this.destroyed = true;
    this.cache.clear();
    this.root.remove();
  }

  // ---- internals --------------------------------------------------------------

  private onScroll(): void {
    if (this.scrollScheduled) return;
    this.scrollScheduled = true;
    requestAnimationFrame(() => {
      this.scrollScheduled = false;
      if (!this.destroyed) this.render(false);
    });
  }

  /**
   * Render the visible window. `force` re-renders even when the range is
   * unchanged (after a height/encoding change). Fetches only when the requested
   * range falls outside the cached one, and never past `indexedLines`.
   */
  private render(force: boolean): void {
    const total = this.opts.totalLines();
    const indexed = Math.min(this.opts.indexedLines(), total);
    const { first, last } = visibleWindow(
      this.root.scrollTop,
      this.root.clientHeight,
      total,
    );
    if (!force && first === this.renderFirst && last === this.renderLast) return;
    this.renderFirst = first;
    this.renderLast = last;

    // Fetch only if the visible window leaves the cached block — but never
    // past `indexed`: a window entirely ahead of the index is pure placeholder,
    // and painting it keeps the "（待索引…）" rows on screen (Task 11 path).
    const needEnd = Math.min(last, indexed);
    if (needEnd > first && (first < this.cacheStart || needEnd > this.cacheEnd)) {
      void this.fetchAndRender(first, needEnd);
      return; // paint after the fetch resolves
    }
    this.paint(first, last, indexed);
  }

  private async fetchAndRender(first: number, needEnd: number): Promise<void> {
    const mySeq = ++this.seq;
    let res: { first: number; lines: string[]; invalidCount: number };
    try {
      res = await this.opts.loadWindow(first, needEnd - first);
    } catch {
      // A failed fetch must not leave `renderFirst/renderLast` claiming the
      // window is painted: the next scroll into the same range would hit the
      // `!force && first===renderFirst && last===renderLast` short-circuit and
      // the view would stay stuck on stale content forever (final-review N-4).
      this.renderFirst = -1;
      this.renderLast = -1;
      return;
    }
    if (this.destroyed || mySeq !== this.seq) return; // superseded by a newer scroll
    this.opts.onInvalidCount?.(res.invalidCount); // spec §5.4 persistent ⚠
    this.cache = new Map();
    for (let i = 0; i < res.lines.length; i++) this.cache.set(res.first + i, res.lines[i]);
    // Derive the cached span from what actually arrived, not from what we
    // asked for: the core clamps a window that runs past EOF, and claiming the
    // requested range would serve phantom cache hits on the next scroll. Use
    // `res.first` (the core's echoed start) as the left bound so a start that
    // differs from our request cannot open a `[first, res.first)` phantom hole.
    const gotEnd = res.first + res.lines.length;
    this.cacheStart = res.first;
    this.cacheEnd = gotEnd;
    // The window may have moved while awaiting; re-derive from current scroll.
    const total = this.opts.totalLines();
    const w = visibleWindow(this.root.scrollTop, this.root.clientHeight, total);
    this.renderFirst = w.first;
    this.renderLast = w.last;
    this.paint(w.first, w.last, Math.min(this.opts.indexedLines(), total));
  }

  /** Build the DOM rows for `[first, last)`, placeholders beyond `indexed`. */
  private paint(first: number, last: number, indexed: number): void {
    this.rows.style.transform = `translateY(${first * ROW_HEIGHT}px)`;
    this.rows.textContent = "";
    const frag = document.createDocumentFragment();
    for (let line = first; line < last; line++) {
      const row = document.createElement("div");
      row.className = "list-row";
      row.style.position = "absolute";
      row.style.top = `${(line - first) * ROW_HEIGHT}px`;
      row.style.height = `${ROW_HEIGHT}px`;
      row.style.whiteSpace = "pre";
      const gutter = document.createElement("span");
      gutter.className = "list-gutter";
      gutter.textContent = String(line + 1); // raw 1-based line number
      const text = document.createElement("span");
      text.className = "list-text";
      if (line >= indexed) {
        text.className += " list-pending";
        text.textContent = "（待索引…）";
      } else {
        text.textContent = this.cache.get(line) ?? "";
      }
      row.append(gutter, text);
      frag.append(row);
    }
    this.rows.append(frag);
  }
}
