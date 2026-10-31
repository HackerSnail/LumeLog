//! Keyboard-shortcut settings modal (spec §5, M3 子项目 C). Owns the DOM and
//! the recording state machine; persistence is delegated to the caller through
//! `onSet`/`onResetAll` so this module never talks to IPC directly.
import { conflictOf, eventToAccel, validate, type ActionId } from "./keymap";
import { isIMEEvent } from "./shell";

export interface KeymapModalOpts {
  actions: { id: ActionId; title: string }[];
  bindings: Partial<Record<ActionId, string>>;
  /** Persist one binding and return the freshly merged table. */
  onSet: (id: ActionId, accel: string | null) => Promise<Partial<Record<ActionId, string>>>;
  /** Reset everything and return the freshly merged table. */
  onResetAll: () => Promise<Partial<Record<ActionId, string>>>;
  onClose: (changed: boolean) => void;
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text = "",
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = text;
  return node;
}

export function openKeymapModal(opts: KeymapModalOpts): void {
  let local: Partial<Record<ActionId, string>> = { ...opts.bindings };
  let changed = false;
  let recording: ActionId | null = null;
  const errors: Partial<Record<ActionId, string>> = {};
  const titleOf = (id: string): string =>
    opts.actions.find((a) => a.id === id)?.title ?? id;

  const overlay = el("div", "keymap-modal");
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  const box = el("div", "keymap-box");
  const title = el("div", "keymap-title", "快捷键设置");
  const note = el(
    "div",
    "keymap-note",
    "点击“录制”后按下新组合键（Esc 取消；需含 Ctrl/Alt，或使用 F1–F24）。F3 / Ctrl+F 等上下文键不可自定义；绑定 Ctrl+W 等系统组合键可能被系统占用；带 Shift 的符号键可能随键盘布局差异。",
  );
  const rows = el("div", "keymap-rows");
  const status = el("div", "keymap-status", "");
  const foot = el("div", "keymap-foot");

  const appRoot = document.getElementById("app");
  const close = (): void => {
    overlay.remove();
    appRoot?.removeAttribute("inert");
    document.removeEventListener("keydown", onCapture, true);
    opts.onClose(changed);
  };

  const apply = async (id: ActionId, accel: string | null): Promise<void> => {
    let next: Partial<Record<ActionId, string>>;
    try {
      next = await opts.onSet(id, accel);
    } catch {
      errors[id] = "保存失败，请重试";
      render();
      return;
    }
    local = { ...next }; // re-sync with the live merge (conflicts may shift)
    changed = true;
    delete errors[id];
    render();
  };

  const render = (): void => {
    rows.textContent = "";
    for (const a of opts.actions) {
      const row = el("div", "keymap-row");
      row.dataset.action = a.id;
      const name = el("span", "keymap-name", a.title);
      const accel = local[a.id] ?? "";
      const acc = el("span", "keymap-accel", accel || "未绑定");
      const err = el("span", "keymap-error", errors[a.id] ?? "");
      const rec = el(
        "button",
        "keymap-record",
        a.id === recording ? "录制中…（Esc 取消）" : "录制",
      );
      rec.addEventListener("click", () => {
        recording = a.id;
        delete errors[a.id];
        render();
      });
      const clr = el("button", "keymap-clear", "清除");
      clr.title = "解绑（仅能从命令面板触发）";
      clr.addEventListener("click", () => void apply(a.id, ""));
      const def = el("button", "keymap-default", "默认");
      def.addEventListener("click", () => void apply(a.id, null));
      row.append(name, acc, err, rec, clr, def);
      rows.append(row);
    }
    if (recording) {
      const btn = rows.querySelector<HTMLButtonElement>(
        `[data-action="${recording}"] .keymap-record`,
      );
      btn?.focus();
    }
  };

  const onCapture = (e: KeyboardEvent): void => {
    // IME composition keys (keyCode 229) must never close the modal or get
    // recorded as a shortcut (Plan 39).
    if (isIMEEvent(e)) return;
    if (!recording) {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      recording = null;
      render();
      return;
    }
    const accel = eventToAccel(e);
    if (!accel) return; // modifier-only: keep waiting
    const self = recording;
    const ok = validate(accel);
    if (!ok.ok) {
      errors[self] = ok.reason;
      render();
      return;
    }
    const other = conflictOf(accel, local, self);
    if (other) {
      errors[self] = `已绑定给《${titleOf(other)}》`;
      render();
      return;
    }
    recording = null;
    void apply(self, accel);
  };

  const resetAll = el("button", "keymap-reset-all", "全部恢复默认");
  resetAll.addEventListener("click", () => {
    void (async () => {
      try {
        local = { ...(await opts.onResetAll()) };
      } catch {
        status.textContent = "恢复默认失败，请重试";
        return;
      }
      for (const k of Object.keys(errors)) delete errors[k as ActionId];
      changed = true;
      status.textContent = "";
      render();
    })();
  });
  const closeBtn = el("button", "keymap-close", "关闭");
  closeBtn.addEventListener("click", () => close());
  foot.append(resetAll, closeBtn);

  overlay.addEventListener("mousedown", (e) => {
    if (e.target === overlay) close();
  });

  box.append(title, note, rows, status, foot);
  overlay.append(box);
  document.body.append(overlay);
  appRoot?.setAttribute("inert", ""); // keep focus/typing out of the app behind
  document.addEventListener("keydown", onCapture, true);
  render();
  closeBtn.focus();
}
