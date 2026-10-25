//! Document tabs (spec §8: tabbed editing, dirty dot, recent files). Owns the
//! tab strip DOM and the id→view map; each document renders through a `TabView`
//! (either the CodeMirror `EditorHandle` or the read-only `ListView`). The app
//! layer drives open/close/switch and holds the rich, kind-specific handles.

/** The minimal surface the tab strip needs from any document view. */
export interface TabView {
  readonly docId: number;
  /** Root element the strip shows/hides as the active tab changes. */
  readonly root: HTMLElement;
  isDirty(): boolean;
  focus(): void;
  destroy(): void;
}

export interface TabInfo {
  docId: number;
  path: string;
}

export interface TabsCallbacks {
  onActivate?: (docId: number) => void;
  onClose?: (docId: number) => void;
  onDirtyChanged?: (docId: number, dirty: boolean) => void;
}

/** Presentation of the yellow "externally changed" dot (headless-testable). */
export function externalDot(on: boolean): { text: string; warn: boolean; title: string } {
  return {
    text: on ? "●" : "",
    warn: on,
    title: on ? "文件已被外部修改" : "",
  };
}

interface TabEntry {
  info: TabInfo;
  view: TabView;
  el: HTMLElement;
  dot: HTMLElement;
  ext: HTMLElement;
}

/** A horizontal tab strip over a shared content host element. */
export class TabStrip {
  private readonly host: HTMLElement;
  private readonly strip: HTMLElement;
  private readonly entries = new Map<number, TabEntry>();
  private readonly order: number[] = [];
  private activeId: number | null = null;
  private readonly cb: TabsCallbacks;

  constructor(host: HTMLElement, cb: TabsCallbacks = {}) {
    this.host = host;
    this.cb = cb;
    this.strip = document.createElement("div");
    this.strip.className = "tabbar";
    host.prepend(this.strip);
  }

  /** Register a document; makes it active and shows its view. */
  add(info: TabInfo, view: TabView): void {
    const el = document.createElement("div");
    el.className = "tab";
    el.title = info.path;
    const dot = document.createElement("span");
    dot.className = "dirty-dot";
    // Yellow marker for "changed on disk" (spec §4.4), independent of dirty.
    const ext = document.createElement("span");
    ext.className = "ext-dot";
    const label = document.createElement("span");
    label.className = "tab-label";
    label.textContent = basename(info.path);
    el.append(dot, ext, label);
    el.addEventListener("click", () => this.activate(info.docId));
    el.addEventListener("auxclick", (e) => {
      if ((e as MouseEvent).button === 1) this.requestClose(info.docId);
    });

    const entry: TabEntry = { info, view, el, dot, ext };
    this.entries.set(info.docId, entry);
    this.order.push(info.docId);
    this.strip.append(el);
    // The view lives in the host, hidden until activated.
    view.root.style.display = "none";
    this.host.append(view.root);
    this.activate(info.docId);
  }

  activate(docId: number): void {
    const target = this.entries.get(docId);
    if (!target) return;
    for (const [id, entry] of this.entries) {
      const on = id === docId;
      entry.el.classList.toggle("active", on);
      entry.view.root.style.display = on ? "" : "none";
    }
    this.activeId = docId;
    target.view.focus();
    this.cb.onActivate?.(docId);
  }

  active(): TabEntry | null {
    return this.activeId != null ? this.entries.get(this.activeId) ?? null : null;
  }

  /** Open docs in tab order — drives the shell's sidebar list. */
  list(): TabInfo[] {
    const out: TabInfo[] = [];
    for (const id of this.order) {
      const info = this.entries.get(id)?.info;
      if (info) out.push(info);
    }
    return out;
  }

  /** Reflect the view's dirty flag onto the tab dot. */
  refreshDirty(docId: number): void {
    const e = this.entries.get(docId);
    if (!e) return;
    const dirty = e.view.isDirty();
    e.dot.textContent = dirty ? "●" : ""; // no CSS dependency (I1)
    e.dot.classList.toggle("visible", dirty);
    this.cb.onDirtyChanged?.(docId, dirty);
  }

  /** Mark a tab as externally modified (yellow dot; independent of dirty). */
  setExternalFlag(docId: number, on: boolean): void {
    const e = this.entries.get(docId);
    if (!e) return;
    const dot = externalDot(on);
    e.ext.textContent = dot.text;
    e.ext.classList.toggle("warn", dot.warn);
    e.ext.title = dot.title;
  }

  get(docId: number): TabView | undefined {
    return this.entries.get(docId)?.view;
  }

  /** Ask the app to close (app decides on save-if-dirty). */
  requestClose(docId: number): void {
    this.cb.onClose?.(docId);
  }

  /** Actually remove + destroy the tab (call after the save decision). */
  remove(docId: number): void {
    const e = this.entries.get(docId);
    if (!e) return;
    e.view.destroy();
    e.el.remove();
    this.entries.delete(docId);
    const oi = this.order.indexOf(docId);
    if (oi >= 0) this.order.splice(oi, 1);
    if (this.activeId === docId) {
      const next = this.order[this.order.length - 1] ?? null;
      this.activeId = next;
      if (next != null) this.activate(next);
      else this.cb.onActivate?.(-1); // no tab left — app clears the status bar
    }
  }
}

function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
