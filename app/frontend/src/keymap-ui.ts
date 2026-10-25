//! Keyboard-shortcut settings modal (spec §5, M3 子项目 C). Owns the DOM and
//! the recording state machine; persistence is delegated to the caller through
//! `onSet`/`onResetAll` so this module never talks to IPC directly.
import { DEFAULT_BINDINGS, conflictOf, eventToAccel, validate, type ActionId } from "./keymap";

export interface KeymapModalOpts {
  actions: { id: ActionId; title: string }[];
  bindings: Partial<Record<ActionId, string>>;
  onSet: (id: ActionId, accel: string | null) => Promise<void>;
  onResetAll: () => Promise<void>;
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
    "点击“录制”后按下新组合键（Esc 取消；需含 Ctrl/Alt，或使用 F1–F24）。F3 / Ctrl+F 等上下文键不可自定义。",
  );
  const rows = el("div", "keymap-rows");
  const status = el("div", "keymap-status", "");
  const foot = el("div", "keymap-foot");

  const close = (): void => {
    overlay.remove();
    document.removeEventListener("keydown", onCapture, true);
    opts.onClose(changed);
  };

  const apply = async (id: ActionId, accel: string | null): Promise<void> => {
    try {
      await opts.onSet(id, accel);
    } catch {
      errors[id] = "保存失败，请重试";
      render();
      return;
    }
    if (accel === null) delete local[id];
    else local[id] = accel;
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
        await opts.onResetAll();
      } catch {
        status.textContent = "恢复默认失败，请重试";
        return;
      }
      local = { ...DEFAULT_BINDINGS };
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
  document.addEventListener("keydown", onCapture, true);
  render();
}
