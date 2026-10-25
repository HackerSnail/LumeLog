//! Script manager modal (Roadmap-3). Pure DOM: all persistence/run/dialog work
//! is injected via `ScriptModalOpts`, so this module never imports IPC and is
//! fully testable in jsdom.

export const SCRIPT_TEMPLATE = "function onLine(line, ctx) {\n  return line;\n}\n";

export interface ScriptEntryLite {
  name: string;
  sizeBytes: number;
  modifiedMs: number;
}
export interface ScriptConsoleLite {
  level: "log" | "info" | "warn" | "error";
  text: string;
}
export interface ScriptProgressLite {
  scanned: number;
  written: number;
}
export interface ScriptStatsLite {
  scanned: number;
  written: number;
  dropped: number;
  cancelled: boolean;
  truncatedConsole: boolean;
  error: string | null;
}

export interface ScriptModalOpts {
  hasDocument: () => boolean;
  defaultOutputBase: () => string;
  list: () => Promise<ScriptEntryLite[]>;
  read: (name: string) => Promise<string>;
  write: (name: string, text: string) => Promise<void>;
  remove: (name: string) => Promise<void>;
  chooseTarget: (base: string) => Promise<string | null>;
  run: (args: {
    code: string;
    targetPath: string;
    onConsole: (m: ScriptConsoleLite) => void;
    onProgress: (p: ScriptProgressLite) => void;
    onDone: (s: ScriptStatsLite) => void;
  }) => Promise<number>;
  cancel: (reqId: number) => Promise<void>;
  confirm: (message: string) => Promise<boolean>;
  onClose: () => void;
}

const MAX_NAME = 64;

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className: string,
  text = "",
): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  n.className = className;
  n.textContent = text;
  return n;
}

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function validName(name: string): string | null {
  const stem = name.endsWith(".js") ? name.slice(0, -3) : null;
  if (stem === null) return "名字需以 .js 结尾";
  if (stem.length === 0 || stem.length > MAX_NAME) return "名字主体需 1–64 个字符";
  if (!/^[A-Za-z0-9_-]+$/.test(stem)) return "仅允许字母、数字、下划线与连字符";
  return null;
}

export function openScriptModal(opts: ScriptModalOpts): void {
  let scripts: ScriptEntryLite[] = [];
  let current: string | null = null;
  let dirty = false;
  let running = false;
  let reqId: number | null = null;

  const appRoot = document.getElementById("app");
  const overlay = el("div", "script-modal");
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  const box = el("div", "script-box");
  const head = el("div", "script-head");
  head.append(el("span", "script-title", "脚本（QuickJS）"));
  const closeBtn = el("button", "script-close", "关闭");
  head.append(closeBtn);

  const body = el("div", "script-body");
  const side = el("div", "script-side");
  const newRow = el("div", "script-new-row");
  const newName = document.createElement("input");
  newName.className = "script-new-name";
  newName.placeholder = "新脚本名.js";
  const newBtn = el("button", "script-new", "新建");
  newRow.append(newName, newBtn);
  const listEl = el("div", "script-list");
  side.append(newRow, listEl);

  const main = el("div", "script-main");
  const toolbar = el("div", "script-toolbar");
  const saveBtn = el("button", "script-save", "保存");
  const delBtn = el("button", "script-delete", "删除");
  const runBtn = el("button", "script-run", "运行…");
  const cancelBtn = el("button", "script-cancel", "取消运行");
  cancelBtn.style.display = "none";
  const status = el("span", "script-status", "");
  toolbar.append(saveBtn, delBtn, runBtn, cancelBtn, status);
  const editor = document.createElement("textarea");
  editor.className = "script-editor";
  editor.spellcheck = false;
  editor.placeholder = "function onLine(line, ctx) { return line; }";
  const consoleEl = el("pre", "script-console");
  consoleEl.setAttribute("aria-live", "polite");
  main.append(toolbar, editor, el("div", "script-console-title", "控制台"), consoleEl);
  body.append(side, main);
  box.append(head, body);
  overlay.append(box);

  const setStatus = (text: string): void => {
    status.textContent = text;
  };

  const appendConsole = (m: ScriptConsoleLite): void => {
    const line = el("div", `script-line script-${m.level}`, m.text);
    consoleEl.append(line);
    consoleEl.scrollTop = consoleEl.scrollHeight;
  };

  const setRunning = (on: boolean): void => {
    running = on;
    editor.disabled = on;
    saveBtn.disabled = on;
    delBtn.disabled = on;
    newBtn.disabled = on;
    runBtn.disabled = on;
    cancelBtn.style.display = on ? "" : "none";
    if (!on) reqId = null;
  };

  const renderList = (): void => {
    listEl.textContent = "";
    if (scripts.length === 0) {
      listEl.append(el("div", "script-empty", "还没有脚本，输入名字后点“新建”"));
      return;
    }
    for (const s of scripts) {
      const row = el("div", "script-row");
      row.dataset.name = s.name;
      if (s.name === current) row.classList.add("active");
      row.append(el("span", "script-name", s.name));
      row.append(el("span", "script-meta", `${fmtSize(s.sizeBytes)}`));
      row.addEventListener("click", () => void selectScript(s.name));
      listEl.append(row);
    }
  };

  const confirmDiscard = async (): Promise<boolean> => {
    if (!dirty) return true;
    return opts.confirm("当前脚本有未保存的修改，是否放弃？");
  };

  const selectScript = async (name: string): Promise<void> => {
    if (running) return;
    if (!(await confirmDiscard())) return;
    try {
      const text = await opts.read(name);
      current = name;
      editor.value = text;
      dirty = false;
      renderList();
      setStatus("");
    } catch (e) {
      setStatus(String(e));
    }
  };

  const refreshScripts = async (): Promise<void> => {
    try {
      scripts = await opts.list();
      renderList();
    } catch (e) {
      setStatus(String(e));
    }
  };

  newBtn.addEventListener("click", () => {
    void (async () => {
      if (running) return;
      const raw = newName.value.trim();
      const err = validName(raw);
      if (err) {
        setStatus(err);
        return;
      }
      if (!(await confirmDiscard())) return;
      try {
        await opts.write(raw, SCRIPT_TEMPLATE);
        current = raw;
        editor.value = SCRIPT_TEMPLATE;
        dirty = false;
        newName.value = "";
        await refreshScripts();
        setStatus(`已创建 ${raw}`);
      } catch (e) {
        setStatus(String(e));
      }
    })();
  });

  saveBtn.addEventListener("click", () => {
    void (async () => {
      if (!current || running) return;
      try {
        await opts.write(current, editor.value);
        dirty = false;
        await refreshScripts();
        setStatus(`已保存 ${current}`);
      } catch (e) {
        setStatus(String(e));
      }
    })();
  });

  delBtn.addEventListener("click", () => {
    void (async () => {
      if (!current || running) return;
      if (!(await opts.confirm(`删除脚本 ${current}？`))) return;
      try {
        await opts.remove(current);
        current = null;
        editor.value = "";
        dirty = false;
        await refreshScripts();
        setStatus("已删除");
      } catch (e) {
        setStatus(String(e));
      }
    })();
  });

  runBtn.addEventListener("click", () => {
    void (async () => {
      if (running) return;
      if (!opts.hasDocument()) {
        setStatus("请先打开一个文档再运行脚本");
        return;
      }
      const target = await opts.chooseTarget(opts.defaultOutputBase());
      if (!target) return;
      consoleEl.textContent = "";
      setRunning(true);
      setStatus("运行中…");
      try {
        reqId = await opts.run({
          code: editor.value,
          targetPath: target,
          onConsole: appendConsole,
          onProgress: (p) => setStatus(`运行中… 已读 ${p.scanned} 行，写出 ${p.written} 行`),
          onDone: (s) => {
            setRunning(false);
            const bits = [`扫描 ${s.scanned}`, `写出 ${s.written}`, `丢弃 ${s.dropped}`];
            if (s.cancelled) bits.push("已取消");
            if (s.truncatedConsole) bits.push("控制台已截断");
            setStatus(bits.join(" · "));
            if (s.error) appendConsole({ level: "error", text: s.error });
          },
        });
      } catch (e) {
        setRunning(false);
        setStatus(String(e));
      }
    })();
  });

  cancelBtn.addEventListener("click", () => {
    if (reqId !== null) {
      void opts.cancel(reqId).catch((e) => setStatus(String(e)));
      setStatus("取消中…");
    }
  });

  editor.addEventListener("input", () => {
    dirty = true;
  });
  editor.addEventListener("keydown", (e) => {
    if (e.key === "Tab") {
      e.preventDefault();
      const start = editor.selectionStart;
      const end = editor.selectionEnd;
      editor.setRangeText("  ", start, end, "end");
      dirty = true;
    }
  });

  const close = (): void => {
    void (async () => {
      if (running) {
        setStatus("脚本运行中，请先取消或等待完成");
        return;
      }
      if (!(await confirmDiscard())) return;
      overlay.remove();
      appRoot?.removeAttribute("inert");
      opts.onClose();
    })();
  };
  closeBtn.addEventListener("click", () => void close());
  overlay.addEventListener("mousedown", (e) => {
    if (e.target === overlay) void close();
  });

  box.append(head, body);
  document.body.append(overlay);
  appRoot?.setAttribute("inert", "");
  renderList();
  setStatus("");
  void refreshScripts();
}
