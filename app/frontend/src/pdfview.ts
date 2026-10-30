// PdfView: pdf.js page rendering for the viewer dock (Plan 22, proposal
// route C). pdfjs-dist is imported dynamically so the main bundle never pays
// for it; the worker URL is a static `?url` import of this module (it only
// loads with the pdf chunk). Rendering state is small on purpose: current
// document + one in-flight render task — a page/zoom change cancels the
// previous task instead of queueing.

import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

/** Page numbers are 1-based; clamp into [1, total] (0/negative total → 1). */
export function clampPage(page: number, total: number): number {
  if (!Number.isFinite(page) || total <= 0) return 1;
  return Math.min(Math.max(Math.trunc(page), 1), Math.trunc(total));
}

/** Zoom steps ×1.25, clamped to [0.5, 4]. */
export function stepZoom(zoom: number, direction: "in" | "out"): number {
  const factor = direction === "in" ? 1.25 : 0.8;
  const next = zoom * factor;
  return Math.min(4, Math.max(0.5, Math.round(next * 1000) / 1000));
}

/** The subset of a pdf.js text item the search/highlight math needs. */
export interface TextItemGeom {
  str: string;
  /** pdf.js text-space transform [a, b, c, d, e, f]. */
  transform: number[];
  width: number;
}

/** All in-item start offsets of `query` (case-insensitive; empty query → []). */
export function matchItem(itemStr: string, query: string): number[] {
  if (!query) return [];
  const hay = itemStr.toLowerCase();
  const needle = query.toLowerCase();
  const starts: number[] = [];
  let i = hay.indexOf(needle);
  while (i >= 0) {
    starts.push(i);
    i = hay.indexOf(needle, i + 1);
  }
  return starts;
}

/**
 * Screen-space (CSS px) rect for a match inside one text item. pdf.js'
 * `Util.transform(viewport.transform, item.transform)` inlined as a pure
 * matrix multiply; the match box is the proportional horizontal slice of the
 * item (monospace approximation — spec §3 known limitation).
 */
export function hitRect(
  item: TextItemGeom,
  viewportTransform: number[],
  dpr: number,
  charStart: number,
  charLen: number,
): { left: number; top: number; width: number; height: number } | null {
  const m = viewportTransform;
  const t = item.transform;
  if (m.length !== 6 || t.length !== 6) return null;
  const tx = [
    m[0]! * t[0]! + m[2]! * t[1]!,
    m[1]! * t[0]! + m[3]! * t[1]!,
    m[0]! * t[2]! + m[2]! * t[3]!,
    m[1]! * t[2]! + m[3]! * t[3]!,
    m[0]! * t[4]! + m[2]! * t[5]! + m[4]!,
    m[1]! * t[4]! + m[3]! * t[5]! + m[5]!,
  ];
  const fontHeight = Math.hypot(tx[2]!, tx[3]!);
  if (fontHeight <= 0 || item.str.length === 0) return null;
  const scale = Math.abs(m[0]!) || 1;
  const itemWidthDev = item.width * scale;
  const charW = itemWidthDev / item.str.length;
  if (!Number.isFinite(charW) || charW <= 0) return null;
  const left = tx[4]! + charW * charStart;
  const width = charW * Math.max(1, charLen);
  const top = tx[5]! - fontHeight;
  return {
    left: left / dpr,
    top: top / dpr,
    width: width / dpr,
    height: fontHeight / dpr,
  };
}

export interface PdfViewOpts {
  /** Surfaces a render-time problem in Chinese (never throws into the host). */
  onError(e: string): void;
}

export interface PdfViewOpts {
  /** Surfaces a render-time problem in Chinese (never throws into the host). */
  onError(e: string): void;
}

interface StoredHit {
  page: number;
  itemIndex: number;
  charStart: number;
  charLen: number;
}

export class PdfView {
  private readonly host: HTMLElement;
  private readonly opts: PdfViewOpts;
  private pageWrap: HTMLElement;
  private canvas: HTMLCanvasElement;
  private hitsLayer: HTMLElement;
  private doc: unknown = null; // pdfjs PDFDocumentProxy (untyped: dynamic import)
  private task: { cancel(): void; promise: Promise<unknown> } | null = null;
  private destroyed = false;
  // Search state (Plan 24).
  private textCache = new Map<number, Array<{ str: string; transform: number[]; width: number }>>();
  private hits: StoredHit[] = [];
  private lastViewportTransform: number[] | null = null;
  private lastDpr = 1;
  private currentPage = 1;

  constructor(host: HTMLElement, opts: PdfViewOpts) {
    this.host = host;
    this.opts = opts;
    this.pageWrap = document.createElement("div");
    this.pageWrap.className = "pdf-page-wrap";
    this.canvas = document.createElement("canvas");
    this.canvas.className = "pdf-canvas";
    this.hitsLayer = document.createElement("div");
    this.hitsLayer.className = "pdf-hits";
    this.pageWrap.append(this.canvas, this.hitsLayer);
    this.host.append(this.pageWrap);
  }

  /** Parse `data`; resolves with the page count. */
  async load(data: ArrayBuffer): Promise<number> {
    if (this.destroyed) return 0;
    this.textCache.clear(); // per-document cache
    this.hits = [];
    this.hitsLayer.textContent = "";
    try {
      const pdfjs = await import("pdfjs-dist");
      pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;
      const doc = await pdfjs.getDocument({ data: new Uint8Array(data.slice(0)) }).promise;
      if (this.destroyed) {
        void doc.cleanup();
        return 0;
      }
      this.doc = doc;
      return doc.numPages;
    } catch (e) {
      this.opts.onError(`PDF 打开失败：${String(e)}`);
      return 0;
    }
  }

  /** Render `page` (1-based) at `scale`; cancels any in-flight render.
   *  Resolves with the on-screen CSS width (fit-width math needs it). */
  async render(page: number, scale: number): Promise<number | null> {
    if (this.destroyed || !this.doc) return null;
    const doc = this.doc as {
      numPages: number;
      getPage(n: number): Promise<{
        getViewport(o: { scale: number }): {
          width: number;
          height: number;
          transform: number[];
        };
        render(o: {
          canvasContext: CanvasRenderingContext2D;
          viewport: { width: number; height: number };
        }): { cancel(): void; promise: Promise<unknown> };
        getTextContent(): Promise<{
          items: Array<{ str?: string; transform?: number[]; width?: number }>;
        }>;
      }>;
      destroy(): Promise<void>;
    };
    this.task?.cancel();
    try {
      const pdfPage = await doc.getPage(clampPage(page, (this.doc as { numPages: number }).numPages));
      if (this.destroyed) return null;
      this.currentPage = clampPage(page, (this.doc as { numPages: number }).numPages);
      this.zoom = scale;
      const dpr = window.devicePixelRatio || 1;
      const viewport = pdfPage.getViewport({ scale: scale * dpr });
      const canvas = this.canvas;
      canvas.width = Math.trunc(viewport.width);
      canvas.height = Math.trunc(viewport.height);
      canvas.style.width = `${viewport.width / dpr}px`;
      canvas.style.height = `${viewport.height / dpr}px`;
      this.pageWrap.style.width = `${viewport.width / dpr}px`;
      this.pageWrap.style.height = `${viewport.height / dpr}px`;
      const ctx = canvas.getContext("2d");
      if (!ctx) {
        this.opts.onError("当前环境不支持 canvas 渲染");
        return null;
      }
      const task = pdfPage.render({ canvasContext: ctx, viewport });
      this.task = task;
      await task.promise;
      this.lastViewportTransform = viewport.transform;
      this.lastDpr = dpr;
      this.drawHits(viewport.transform, dpr);
      return viewport.width / dpr;
    } catch (e) {
      // A cancelled render (page/zoom changed mid-flight) rejects too — only
      // real failures surface.
      if (!this.destroyed && String(e) !== "Rendering cancelled") {
        this.opts.onError(`PDF 渲染失败：${String(e)}`);
      }
      return null;
    } finally {
      this.task = null;
    }
  }

  /** Cached `getTextContent()` items for a page (search input). */
  private async textItems(page: number): Promise<Array<{ str: string; transform: number[]; width: number }>> {
    const cached = this.textCache.get(page);
    if (cached) return cached;
    const doc = this.doc as {
      getPage(n: number): Promise<{
        getTextContent(): Promise<{
          items: Array<{ str?: string; transform?: number[]; width?: number }>;
        }>;
      }>;
    } | null;
    if (!doc) return [];
    const pdfPage = await doc.getPage(page);
    const content = await pdfPage.getTextContent();
    const items = content.items
      .filter((it) => typeof it.str === "string" && it.str.length > 0 && Array.isArray(it.transform))
      .map((it) => ({ str: it.str as string, transform: it.transform as number[], width: it.width ?? 0 }));
    this.textCache.set(page, items);
    return items;
  }

  /**
   * Search the whole document (case-insensitive, per text item — spec §1).
   * Replaces any previous hits; resolves with the hit count.
   */
  async searchAll(
    query: string,
    onProgress?: (done: number, total: number) => void,
  ): Promise<number> {
    if (this.destroyed || !this.doc) return 0;
    const total = (this.doc as { numPages: number }).numPages;
    const hits: StoredHit[] = [];
    const q = query.trim();
    if (q) {
      for (let p = 1; p <= total; p++) {
        const items = await this.textItems(p);
        for (let idx = 0; idx < items.length; idx++) {
          for (const start of matchItem(items[idx]!.str, q)) {
            hits.push({ page: p, itemIndex: idx, charStart: start, charLen: q.length });
          }
        }
        onProgress?.(p, total);
      }
    }
    this.hits = hits;
    this.drawHits(this.lastViewportTransform, this.lastDpr);
    return hits.length;
  }

  /** Jump to hit `index` (clamped): page switch + render + scroll into view. */
  async gotoHit(index: number): Promise<void> {
    if (this.hits.length === 0) return;
    const i = Math.min(Math.max(index, 0), this.hits.length - 1);
    const hit = this.hits[i]!;
    if (hit.page !== this.currentPage) {
      await this.render(hit.page, this.zoom);
    }
    this.drawHits(this.lastViewportTransform, this.lastDpr, i);
    const rect = this.hitScreenRect(hit);
    if (rect) {
      const scroller = this.host;
      scroller.scrollTop = Math.max(0, rect.top + scroller.scrollTop - scroller.clientHeight / 2 + rect.height / 2);
    }
  }

  get hitCount(): number {
    return this.hits.length;
  }

  clearSearch(): void {
    this.hits = [];
    this.hitsLayer.textContent = "";
  }

  private zoom = 1;

  /** Recompute hit boxes for the current page into the overlay. */
  private drawHits(transform: number[] | null, dpr: number, current = -1): void {
    this.hitsLayer.textContent = "";
    if (!transform) return;
    this.hits.forEach((hit, i) => {
      if (hit.page !== this.currentPage) return;
      const items = this.textCache.get(hit.page);
      const item = items?.[hit.itemIndex];
      if (!item) return;
      const rect = hitRect(item, transform, dpr, hit.charStart, hit.charLen);
      if (!rect) return;
      const div = document.createElement("div");
      div.className = i === current ? "pdf-hit pdf-hit-current" : "pdf-hit";
      div.style.left = `${rect.left}px`;
      div.style.top = `${rect.top}px`;
      div.style.width = `${rect.width}px`;
      div.style.height = `${rect.height}px`;
      this.hitsLayer.append(div);
    });
  }

  /** Screen rect of a hit at the LAST rendered viewport (gotoHit uses it). */
  private hitScreenRect(hit: StoredHit) {
    if (!this.lastViewportTransform) return null;
    const items = this.textCache.get(hit.page);
    const item = items?.[hit.itemIndex];
    if (!item) return null;
    return hitRect(item, this.lastViewportTransform, this.lastDpr, hit.charStart, hit.charLen);
  }

  destroy(): void {
    this.destroyed = true;
    this.task?.cancel();
    (this.doc as { destroy?(): Promise<void> } | null)?.destroy?.().catch(() => {});
    this.doc = null;
    this.textCache.clear();
    this.pageWrap.remove();
  }
}
