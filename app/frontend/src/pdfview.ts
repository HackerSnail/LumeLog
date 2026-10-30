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

export interface PdfViewOpts {
  /** Surfaces a render-time problem in Chinese (never throws into the host). */
  onError(e: string): void;
}

export class PdfView {
  private readonly host: HTMLElement;
  private readonly opts: PdfViewOpts;
  private canvas: HTMLCanvasElement;
  private doc: unknown = null; // pdfjs PDFDocumentProxy (untyped: dynamic import)
  private task: { cancel(): void; promise: Promise<unknown> } | null = null;
  private destroyed = false;

  constructor(host: HTMLElement, opts: PdfViewOpts) {
    this.host = host;
    this.opts = opts;
    this.canvas = document.createElement("canvas");
    this.canvas.className = "pdf-canvas";
    this.host.append(this.canvas);
  }

  /** Parse `data`; resolves with the page count. */
  async load(data: ArrayBuffer): Promise<number> {
    if (this.destroyed) return 0;
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
      getPage(n: number): Promise<{
        getViewport(o: { scale: number }): { width: number; height: number };
        render(o: {
          canvasContext: CanvasRenderingContext2D;
          viewport: { width: number; height: number };
        }): { cancel(): void; promise: Promise<unknown> };
      }>;
      destroy(): Promise<void>;
    };
    this.task?.cancel();
    try {
      const pdfPage = await doc.getPage(clampPage(page, (this.doc as { numPages: number }).numPages));
      if (this.destroyed) return null;
      const dpr = window.devicePixelRatio || 1;
      const viewport = pdfPage.getViewport({ scale: scale * dpr });
      const canvas = this.canvas;
      canvas.width = Math.trunc(viewport.width);
      canvas.height = Math.trunc(viewport.height);
      canvas.style.width = `${viewport.width / dpr}px`;
      canvas.style.height = `${viewport.height / dpr}px`;
      const ctx = canvas.getContext("2d");
      if (!ctx) {
        this.opts.onError("当前环境不支持 canvas 渲染");
        return null;
      }
      const task = pdfPage.render({ canvasContext: ctx, viewport });
      this.task = task;
      await task.promise;
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

  destroy(): void {
    this.destroyed = true;
    this.task?.cancel();
    (this.doc as { destroy?(): Promise<void> } | null)?.destroy?.().catch(() => {});
    this.doc = null;
    this.canvas.remove();
  }
}
