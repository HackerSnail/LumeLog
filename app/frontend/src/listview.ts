//! Read-only virtual list for large files (spec §4.3, >200 MB path). Renders a
//! fixed-row-height window (visible ± `OVERSCAN` lines) and requests exactly one
//! `doc_chunk` per window move — the whole file is never resident. Rows are
//! absolutely positioned against a full-height spacer, so a 1 GiB log scrolls
//! smoothly while only a few thousand line strings exist at a time.
//!
//! Two view modes (M3 spec §4.2):
//!   * **contiguous** (default) — row slot k === original line k; text fetched
//!     by a windowed `loadWindow`; optionally overlays `filter_run` highlight
//!     ranges on matched lines (`highlight` on, `hideNon` off).
//!   * **subset** (filtered) — the list shows only the query's kept lines; slot
//!     k maps to `hits[k].line` (original number preserved in the gutter); text
//!     fetched per visible window via `loadLines` (never the whole match set).
//!
//! This is a *view*, not an editor: `isDirty()` is always false, there is no
//! cursor, and the only interaction is scroll + `scrollToLine`/`scrollToHit`.

import type { TabView } from "./tabs";
import type { Cu16Range, IndexedText, LineHit } from "./ipc";

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
  /** Fetch one window of decoded lines (contiguous mode). Injected so the view
   *  has no IPC import. */
  loadWindow: (
    start: number,
    count: number,
  ) => Promise<{ first: number; lines: string[]; invalidCount: number }>;
  /** Fetch decoded text for a sparse set of original line indices (subset mode,
   *  `doc_lines_at`). Absent → subset mode is unavailable. Reports the batch's
   *  unmappable count so the ⚠ (spec §5.4) stays live in the filtered view. */
  loadLines?: (indices: number[]) => Promise<{ lines: IndexedText[]; invalidCount: number }>;
  /** Report decoded-but-unmappable counts for the live window (spec §5.4). */
  onInvalidCount?: (count: number) => void;
  /** Report scroll position (px) and max scroll after each scroll (tail follow). */
  onScrollTop?: (top: number, maxScroll: number) => void;
}

/** True when the viewport is within `threshold` px of the bottom (0 max => true). */
export function isNearBottom(scrollTop: number, maxScroll: number, threshold = 48): boolean {
  const max = Math.max(0, maxScroll);
  return max - scrollTop <= threshold;
}

/**
 * Pure window math (unit-tested in `listview.test.ts`, no DOM). Returns the
 * half-open range `[first, last)` of *slots* to render for a given scroll
 * position, clamped to `[0, totalSlots)`.
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

/** A styled text segment: `hl` → wrap in a highlight span when painting. */
export interface TextSeg {
  text: string;
  hl: boolean;
}

/**
 * Pure (no DOM, unit-tested): split a line into styled segments by its UTF-16
 * highlight ranges. Ranges are cu16 (JS `String.slice` indices), so this is a
 * direct slice. Out-of-range/empty spans are dropped; overlapping/adjacent
 * spans are merged so no character is emitted twice and highlight never splits
 * a surrogate pair (backend ranges are char-aligned).
 */
export function sliceLineTextByRanges(text: string, ranges: Cu16Range[]): TextSeg[] {
  const segs: Array<[number, number]> = [];
  for (const r of ranges) {
    const s = Math.max(0, Math.min(r.cu16Start, text.length));
    const e = Math.max(0, Math.min(r.cu16Start + r.cu16Len, text.length));
    if (e > s) segs.push([s, e]);
  }
  if (segs.length === 0) return [{ text, hl: false }];
  segs.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const merged: Array<[number, number]> = [];
  for (const [s, e] of segs) {
    const last = merged[merged.length - 1];
    if (last && s <= last[1]) last[1] = Math.max(last[1], e);
    else merged.push([s, e]);
  }
  const out: TextSeg[] = [];
  let cursor = 0;
  for (const [s, e] of merged) {
    if (s > cursor) out.push({ text: text.slice(cursor, s), hl: false });
    out.push({ text: text.slice(s, e), hl: true });
    cursor = e;
  }
  if (cursor < text.length) out.push({ text: text.slice(cursor), hl: false });
  return out;
}

export class ListView implements TabView {
  readonly docId: number;
  readonly root: HTMLElement;

  private readonly spacer: HTMLElement;
  private readonly rows: HTMLElement;
  private readonly opts: ListViewOpts;

  /** Cache of original line index → text; shared by both view modes. */
  private cache = new Map<number, string>();
  /** Contiguous-mode cached span `[cacheStart, cacheEnd)`. */
  private cacheStart = -1;
  private cacheEnd = -1;
  /** Last `[first, last)` slots actually rendered — skip work when unmoved. */
  private renderFirst = -1;
  private renderLast = -1;
  /** Guards against overlapping async fetches racing a fast scroll. */
  private seq = 0;
  private destroyed = false;
  private scrollScheduled = false;
  private flashIndex = -1;
  private flashTimer = 0;

  // ---- filter/subset view state (M3) ----
  private hits: LineHit[] | null = null;
  private hideNon = false;
  private hitMap: Map<number, Cu16Range[]> | null = null;

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

  /** Number of slots in the current view (subset length, or full line count). */
  private viewTotal(): number {
    if (this.hits && this.hideNon) return this.hits.length;
    return this.opts.totalLines();
  }

  /** Re-measure total height and (re)render — call after open + on index progress. */
  refresh(): void {
    const total = this.viewTotal();
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

  /**
   * Apply a `filter_run` result. `hits=null` clears the query (plain list).
   * Otherwise: `hideNon` → render only kept lines (subset mode); `highlight` →
   * overlay match ranges on the visible lines (both modes). The hit array is
   * retained for F3/Shift+F3 cycling regardless of `hideNon`.
   */
  setView(hits: LineHit[] | null, hideNon: boolean, highlight: boolean): void {
    this.hits = hits;
    this.hideNon = hits !== null && hideNon;
    this.hitMap = hits && highlight ? new Map(hits.map((h) => [h.line, h.ranges])) : null;
    // Switching mode changes total + slot↔line mapping; force a fresh window.
    this.renderFirst = -1;
    this.renderLast = -1;
    this.refresh();
  }

  /** Current vertical scroll offset (px) — kept across an external reload. */
  scrollTop(): number {
    return this.root.scrollTop;
  }

  /** Restore a scroll offset after a reload; the scroll event repaints. */
  setScrollTop(px: number): void {
    this.root.scrollTop = Math.max(0, px);
  }

  /** Highest scrollable offset (0 when the content fits). */
  maxScrollTop(): number {
    return Math.max(0, this.root.scrollHeight - this.root.clientHeight);
  }

  /** Pin the viewport to the newest lines (tail follow). */
  scrollToBottom(): void {
    // A hidden tab has no layout (clientHeight 0); scrolling it would clamp to 0
    // and lose the pinned position when it becomes visible again.
    if (this.root.clientHeight === 0) return;
    this.root.scrollTop = this.maxScrollTop();
    this.render(true);
  }

  /** Bring a slot into view: subset mode uses the slot index, plain mode the
   *  line index. Used by F3/Shift+F3 (caller tracks which hit index is current). */
  scrollToHit(hitIndex: number): void {
    if (!this.hits) return;
    const slot = this.hideNon
      ? Math.min(Math.max(0, hitIndex), this.hits.length - 1)
      : this.hits[Math.min(Math.max(0, hitIndex), this.hits.length - 1)].line;
    this.root.scrollTop = slot * ROW_HEIGHT;
    this.render(true);
  }

  /** Bring an absolute (0-based) line into view; keeps it near the top. In
   *  subset mode the original line maps to its nearest visible slot (spec §4.2:
   *  jump/F3 locate by `hits`, not raw line count). */
  scrollToLine(zeroBasedLine: number): void {
    if (this.hits && this.hideNon && this.hits.length) {
      this.root.scrollTop = this.slotForLine(zeroBasedLine) * ROW_HEIGHT;
      this.render(true);
      return;
    }
    const total = this.opts.totalLines();
    const line = Math.min(Math.max(0, zeroBasedLine), Math.max(0, total - 1));
    this.root.scrollTop = line * ROW_HEIGHT;
    this.render(true);
  }

  /** Subset mode: first slot whose original line is >= `line` (clamped); hits
   *  are ascending by original line, so a binary search is exact. */
  private slotForLine(line: number): number {
    const hits = this.hits as LineHit[];
    let lo = 0;
    let hi = hits.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (hits[mid].line < line) lo = mid + 1;
      else hi = mid;
    }
    return Math.min(lo, hits.length - 1);
  }

  /**
   * Scroll `index0` into view and flash-highlight the row once — the CanPanel
   * row→list linkage target (spec §7.4). The highlight is applied inside `paint`
   * (the row may still be an async fetch when this is called) and cleared on a
   * timer, so a fast scroll never strands the flash class on the wrong row.
   */
  revealLine(index0: number): void {
    this.scrollToLine(index0);
    this.flashIndex = index0;
    this.render(true);
    window.clearTimeout(this.flashTimer);
    this.flashTimer = window.setTimeout(() => {
      this.flashIndex = -1;
      if (!this.destroyed) this.render(true);
    }, 700);
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
    window.clearTimeout(this.flashTimer);
    this.cache.clear();
    this.root.remove();
  }

  // ---- internals --------------------------------------------------------------

  private onScroll(): void {
    this.opts.onScrollTop?.(this.root.scrollTop, this.maxScrollTop());
    if (this.scrollScheduled) return;
    this.scrollScheduled = true;
    requestAnimationFrame(() => {
      this.scrollScheduled = false;
      if (!this.destroyed) this.render(false);
    });
  }

  /**
   * Render the visible window of slots. `force` re-renders even when the range
   * is unchanged (after a height/encoding/mode change). Subset mode fetches per
   * sparse window via `loadLines`; contiguous mode via a windowed `loadWindow`.
   */
  private render(force: boolean): void {
    const subset = this.hits !== null && this.hideNon;
    const total = this.viewTotal();
    const { first, last } = visibleWindow(this.root.scrollTop, this.root.clientHeight, total);
    if (!force && first === this.renderFirst && last === this.renderLast) return;
    this.renderFirst = first;
    this.renderLast = last;

    if (subset) {
      const hits = this.hits as LineHit[];
      const need: number[] = [];
      for (let k = first; k < last; k++) {
        const line = hits[k].line;
        if (!this.cache.has(line)) need.push(line);
      }
      if (need.length && this.opts.loadLines) {
        void this.fetchSubsetAndRender(need);
        return;
      }
      this.paintSubset(first, last, hits);
      return;
    }

    const indexed = Math.min(this.opts.indexedLines(), total);
    const needEnd = Math.min(last, indexed);
    if (needEnd > first && (first < this.cacheStart || needEnd > this.cacheEnd)) {
      void this.fetchAndRender(first, needEnd);
      return;
    }
    this.paint(first, last, indexed);
  }

  private async fetchSubsetAndRender(indices: number[]): Promise<void> {
    const mySeq = ++this.seq;
    const load = this.opts.loadLines;
    if (!load) return;
    let res: { lines: IndexedText[]; invalidCount: number };
    try {
      res = await load(indices);
    } catch {
      this.renderFirst = -1;
      this.renderLast = -1;
      return;
    }
    if (this.destroyed || mySeq !== this.seq) return;
    this.opts.onInvalidCount?.(res.invalidCount); // spec §5.4 persistent ⚠ in filtered view
    for (const it of res.lines) this.cache.set(it.index, it.text);
    const total = this.viewTotal();
    const w = visibleWindow(this.root.scrollTop, this.root.clientHeight, total);
    this.renderFirst = w.first;
    this.renderLast = w.last;
    if (this.hits && this.hideNon) this.paintSubset(w.first, w.last, this.hits);
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
    const gotEnd = res.first + res.lines.length;
    this.cacheStart = res.first;
    this.cacheEnd = gotEnd;
    const total = this.viewTotal();
    const w = visibleWindow(this.root.scrollTop, this.root.clientHeight, total);
    this.renderFirst = w.first;
    this.renderLast = w.last;
    this.paint(w.first, w.last, Math.min(this.opts.indexedLines(), total));
  }

  /** Build a `.list-text` span's children, honouring highlight ranges when set. */
  private renderText(textEl: HTMLElement, text: string, ranges: Cu16Range[] | null): void {
    if (!ranges || ranges.length === 0) {
      textEl.textContent = text;
      return;
    }
    textEl.textContent = "";
    for (const seg of sliceLineTextByRanges(text, ranges)) {
      if (seg.hl) {
        const s = document.createElement("span");
        s.className = "line-hl";
        s.textContent = seg.text;
        textEl.append(s);
      } else {
        textEl.append(seg.text);
      }
    }
  }

  /** Build the DOM rows for slots `[first, last)` in subset mode. */
  private paintSubset(first: number, last: number, hits: LineHit[]): void {
    this.rows.style.transform = `translateY(${first * ROW_HEIGHT}px)`;
    this.rows.textContent = "";
    const frag = document.createDocumentFragment();
    for (let k = first; k < last; k++) {
      const hit = hits[k];
      const row = document.createElement("div");
      row.className = "list-row";
      if (hit.line === this.flashIndex) row.className += " list-flash";
      row.style.position = "absolute";
      row.style.top = `${(k - first) * ROW_HEIGHT}px`;
      row.style.height = `${ROW_HEIGHT}px`;
      row.style.whiteSpace = "pre";
      const gutter = document.createElement("span");
      gutter.className = "list-gutter";
      gutter.textContent = String(hit.line + 1); // original 1-based line number
      const text = document.createElement("span");
      text.className = "list-text";
      this.renderText(text, this.cache.get(hit.line) ?? "", this.hitMap?.get(hit.line) ?? null);
      row.append(gutter, text);
      frag.append(row);
    }
    this.rows.append(frag);
  }

  /** Build the DOM rows for `[first, last)` in contiguous mode. */
  private paint(first: number, last: number, indexed: number): void {
    this.rows.style.transform = `translateY(${first * ROW_HEIGHT}px)`;
    this.rows.textContent = "";
    const frag = document.createDocumentFragment();
    for (let line = first; line < last; line++) {
      const row = document.createElement("div");
      row.className = "list-row";
      if (line === this.flashIndex) row.className += " list-flash";
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
        this.renderText(text, this.cache.get(line) ?? "", this.hitMap?.get(line) ?? null);
      }
      row.append(gutter, text);
      frag.append(row);
    }
    this.rows.append(frag);
  }
}
