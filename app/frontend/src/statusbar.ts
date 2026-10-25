//! Status bar (spec §8): encoding dropdown, EOL + language selects, dirty
//! indicator, line/col. The encoding switch re-decodes the document in place
//! (see `main.ts` `changeEncoding` → `doc_set_encoding`); this widget only
//! reports the choice — it never mutates the editor itself.

import { ENCODINGS, EOL_OPTIONS } from "./encoding-menu";
import { AVAILABLE_MODES, type ModeChoice } from "./language";

export interface StatusbarCallbacks {
  onEncoding?: (label: string) => void;
  onEol?: (eol: string) => void;
  onMode?: (mode: ModeChoice) => void;
  /** Tail follow toggle (read-only docs only). */
  onFollow?: (on: boolean) => void;
}

const EOL_CHOICES = EOL_OPTIONS;

export class Statusbar {
  private readonly root: HTMLElement;
  private readonly encSel: HTMLSelectElement;
  private readonly eolSel: HTMLSelectElement;
  private readonly modeSel: HTMLSelectElement;
  private readonly dirtyEl: HTMLElement;
  private readonly posEl: HTMLElement;
  private readonly invalidEl: HTMLElement;
  private readonly watchEl: HTMLElement;
  private readonly followBtn: HTMLButtonElement;

  constructor(root: HTMLElement, cb: StatusbarCallbacks = {}) {
    this.root = root;
    root.className = "statusbar";

    this.encSel = mkSelect("encoding", ENCODINGS.map((e) => e.label));
    this.encSel.addEventListener("change", () => cb.onEncoding?.(this.encSel.value));

    this.eolSel = mkSelect("eol", [...EOL_CHOICES]);
    this.eolSel.addEventListener("change", () => cb.onEol?.(this.eolSel.value));

    this.modeSel = mkSelect(
      "mode",
      AVAILABLE_MODES.map((m) => m.toUpperCase()),
    );
    this.modeSel.addEventListener("change", () =>
      cb.onMode?.(AVAILABLE_MODES[this.modeSel.selectedIndex]),
    );

    this.dirtyEl = mkSpan("dirty-flag", "");
    this.posEl = mkSpan("doc-pos", "");
    // Persistent ⚠ beside the encoding name (spec §5.4): a decoded window that
    // hit unmappable bytes stays flagged while the doc is open, so a wrong
    // encoding is discoverable — not a one-shot toast that scrolls away.
    this.invalidEl = mkSpan("status-invalid", "");
    this.invalidEl.title = "当前编码无法映射部分字符（以 ? 显示），可能是编码选错了";
    this.invalidEl.style.display = "none";

    // External-watch degradation (spec §4.4): hidden while watching works.
    this.watchEl = mkSpan("status-watch", "");
    this.watchEl.style.display = "none";

    // Tail follow (Roadmap): shown for read-only docs by `setFollowAvailable`.
    this.followBtn = document.createElement("button");
    this.followBtn.type = "button";
    this.followBtn.className = "status-follow";
    this.followBtn.textContent = "跟随尾部";
    this.followBtn.title = "自动滚动到最新追加的行";
    this.followBtn.style.display = "none";
    this.followBtn.addEventListener("click", () => {
      const on = !this.followBtn.classList.contains("active");
      this.setFollow(on);
      cb.onFollow?.(on);
    });

    root.append(
      mkLabel("编码"), this.encSel, this.invalidEl,
      mkLabel("行尾"), this.eolSel,
      mkLabel("语法"), this.modeSel,
      this.followBtn, this.watchEl, this.dirtyEl, this.posEl,
    );
  }

  /** Show/hide the tail-follow toggle (read-only docs only). */
  setFollowAvailable(on: boolean): void {
    this.followBtn.style.display = on ? "" : "none";
    if (!on) this.setFollow(false);
  }

  /** Reflect the follow state on the button. */
  setFollow(on: boolean): void {
    this.followBtn.classList.toggle("active", on);
    this.followBtn.setAttribute("aria-pressed", on ? "true" : "false");
  }

  /** Report external-watch health; `degraded` shows a warn chip + tooltip. */
  setWatch(state: "ok" | "degraded", reason?: string): void {
    const bad = state === "degraded";
    this.watchEl.textContent = bad ? "外部监听已降级" : "";
    this.watchEl.title = bad ? (reason ?? "") : "";
    this.watchEl.style.display = bad ? "" : "none";
  }

  setEncoding(label: string): void {
    const i = ENCODINGS.findIndex((e) => e.label === label);
    if (i >= 0) {
      // Drop any transient "raw" option from a previous unknown label.
      if (this.encSel.dataset.raw === "1") {
        this.encSel.querySelector("option[data-raw]")?.remove();
        this.encSel.dataset.raw = "";
      }
      this.encSel.selectedIndex = i;
    } else {
      // Unknown (not one of the 9): show it verbatim rather than lying UTF-8.
      let opt = this.encSel.querySelector("option[data-raw]") as HTMLOptionElement | null;
      if (!opt) {
        opt = document.createElement("option");
        opt.dataset.raw = "1";
        opt.disabled = true;
        this.encSel.append(opt);
      }
      opt.value = label;
      opt.textContent = label;
      this.encSel.value = label;
      this.encSel.dataset.raw = "1";
    }
  }

  setEol(eol: string): void {
    const v = eol.toUpperCase();
    const i = EOL_CHOICES.findIndex((e) => e === v);
    this.eolSel.selectedIndex = i >= 0 ? i : 0;
  }

  setDirty(dirty: boolean): void {
    this.dirtyEl.textContent = dirty ? "● 未保存" : "";
    this.dirtyEl.classList.toggle("visible", dirty);
  }

  /** Show/hide the persistent ⚠ beside the encoding for a decoded window. */
  setInvalidCount(count: number): void {
    if (count > 0) {
      this.invalidEl.textContent = `⚠ ${count}`;
      this.invalidEl.style.display = "";
    } else {
      this.invalidEl.textContent = "";
      this.invalidEl.style.display = "none";
    }
  }

  setPos(line: number, col: number, lines: number): void {
    this.posEl.textContent = `行 ${line}/${lines} · 列 ${col}`;
  }

  /** Read-only view: no cursor, so show the line tally instead of a position. */
  setReadonly(lines: number): void {
    this.setDirty(false);
    this.posEl.textContent = `只读 · 共 ${lines} 行`;
  }

  clear(): void {
    this.setDirty(false);
    this.setInvalidCount(0);
    this.setFollowAvailable(false); // no document => no tail-follow toggle
    this.posEl.textContent = "";
  }

  /** Dim the bar while a blocking op (open/save/switch) is in flight. */
  setBusy(busy: boolean): void {
    this.root.classList.toggle("busy", busy);
  }
}

function mkSelect(cls: string, options: string[]): HTMLSelectElement {
  const sel = document.createElement("select");
  sel.className = cls;
  for (const o of options) {
    const opt = document.createElement("option");
    opt.value = o;
    opt.textContent = o;
    sel.append(opt);
  }
  return sel;
}

function mkSpan(cls: string, text: string): HTMLElement {
  const s = document.createElement("span");
  s.className = cls;
  s.textContent = text;
  return s;
}

function mkLabel(text: string): HTMLElement {
  const l = document.createElement("span");
  l.className = "status-label";
  l.textContent = text;
  return l;
}
