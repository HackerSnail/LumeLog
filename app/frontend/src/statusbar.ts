//! Status bar (spec §8): encoding dropdown, EOL + language selects, dirty
//! indicator, line/col. The encoding switch re-opens the document under the new
//! label (see `main.ts` `reopenWithEncoding`); this widget only reports the
//! choice — it never mutates the editor itself.

import { ENCODINGS, EOL_OPTIONS } from "./encoding-menu";
import { AVAILABLE_MODES, type ModeChoice } from "./language";

export interface StatusbarCallbacks {
  onEncoding?: (label: string) => void;
  onEol?: (eol: string) => void;
  onMode?: (mode: ModeChoice) => void;
}

const EOL_CHOICES = EOL_OPTIONS;

export class Statusbar {
  private readonly root: HTMLElement;
  private readonly encSel: HTMLSelectElement;
  private readonly eolSel: HTMLSelectElement;
  private readonly modeSel: HTMLSelectElement;
  private readonly dirtyEl: HTMLElement;
  private readonly posEl: HTMLElement;

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

    root.append(
      mkLabel("编码"), this.encSel,
      mkLabel("行尾"), this.eolSel,
      mkLabel("语法"), this.modeSel,
      this.dirtyEl, this.posEl,
    );
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

  setPos(line: number, col: number, lines: number): void {
    this.posEl.textContent = `行 ${line}/${lines} · 列 ${col}`;
  }

  clear(): void {
    this.setDirty(false);
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
