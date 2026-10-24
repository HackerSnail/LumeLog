//! App wiring: layout, open/save/encoding/search flows over the Task 8 IPC.
//! Edit mode is ≤200MB (spec §4.3/A1); oversized files are refused (Task 10
//! adds the read-only virtual list). GUI acceptance (`tauri dev`, GBK no-mojibake
//! roundtrip, drag-drop, recent-20) is the Task 11 pass; the logic below is
//! headless-checked by `tsc --noEmit` (strict) + `vitest`.

import { open, ask } from "@tauri-apps/plugin-dialog";
import {
  docOpen,
  docClose,
  docSave,
  docSetEncoding,
  loadAllText,
  searchRun,
  IpcError,
  type DocInfo,
  type MatchJson,
} from "./ipc";
import { openEditable, type EditorHandle } from "./editor";
import { TabStrip } from "./tabs";
import { Statusbar } from "./statusbar";
import { extOf } from "./encoding-menu";
import type { ModeChoice } from "./language";

/** spec §4.3 / A1: ≤200MB edits in CodeMirror, larger is read-only (Task 10). */
const EDIT_THRESHOLD = 200 * 1024 * 1024;

interface DocState {
  path: string;
  encoding: string;
  eol: string;
  totalLines: number;
}

const docStates = new Map<number, DocState>();
let busy = false;

const tabsEl = document.createElement("div");
tabsEl.id = "tabs";
const statusEl = document.createElement("div");
statusEl.id = "status";

const tabs = new TabStrip(tabsEl, {
  onActivate: (id) => (id < 0 ? status.clear() : syncStatus(id)),
  onClose: (id) => void handleClose(id),
  onDirtyChanged: (id, dirty) => {
    if (id === tabs.active()?.info.docId) status.setDirty(dirty);
  },
});
const status = new Statusbar(statusEl, {
  onEncoding: (label) => void changeEncoding(label),
  onEol: (eol) => {
    const a = tabs.active();
    if (a) {
      a.handle.setEol(eol.toUpperCase());
      docStates.get(a.info.docId)!.eol = eol.toUpperCase();
      tabs.refreshDirty(a.info.docId);
    }
  },
  onMode: (mode) => {
    const a = tabs.active();
    if (a) applyMode(a.info.docId, a.handle, mode);
  },
});

function layout(): void {
  const root = document.getElementById("root");
  if (!root) return;
  root.textContent = "";
  const bar = document.createElement("div");
  bar.className = "appbar";
  const openBtn = document.createElement("button");
  openBtn.textContent = "打开文件";
  openBtn.addEventListener("click", () => void openFile());
  const searchBtn = document.createElement("button");
  searchBtn.textContent = "搜索";
  searchBtn.addEventListener("click", () => openSearchFlow());
  bar.append(openBtn, searchBtn);

  const toast = document.createElement("div");
  toast.id = "toast";
  toast.className = "toast";

  const panel = document.createElement("div");
  panel.id = "search-panel";
  panel.className = "search-panel";
  panel.style.display = "none";

  root.append(bar, tabsEl, panel, statusEl, toast);
}

async function openFile(): Promise<void> {
  const picked = await open({
    multiple: false,
    filters: [{ name: "日志/文本", extensions: ["*"] }],
  });
  if (typeof picked === "string") await openPath(picked);
}

async function openPath(path: string): Promise<void> {
  if (busy) return;
  setBusy(true);
  try {
    const info = await docOpen(path, () => {});
    if (info.size > EDIT_THRESHOLD) {
      // Release the shell-side doc we just opened to index it (I5), then tell
      // the user the read-only view is not available yet (Task 10).
      void docClose(info.docId);
      toastMsg("该文件超过 200MB，只读大文件视图将在后续版本提供（当前仅支持编辑模式）");
      return;
    }
    await mountDoc(info, path);
  } catch (err) {
    reportError(err);
  } finally {
    setBusy(false);
  }
}

async function mountDoc(info: DocInfo, path: string): Promise<void> {
  const total = info.totalLines ?? info.lineCountEstimate;
  const handle = await openEditable({
    parent: tabsEl,
    info,
    ext: extOf(path),
    onDirty: () => tabs.refreshDirty(info.docId),
    onInvalid: (n) => {
      if (n > 0) toastMsg(`解码时发现 ${n} 处无法映射字符（以 ? 显示）`);
    },
  });
  docStates.set(info.docId, {
    path,
    encoding: info.encoding,
    eol: info.eol || "LF",
    totalLines: total,
  });
  tabs.add({ docId: info.docId, path }, handle);
  trackCursor(handle);
  syncStatus(info.docId);
}

function trackCursor(handle: EditorHandle): void {
  // Light listeners keep line/col in sync; the full selection extension is a
  // Task 11 GUI polish.
  handle.view.dom.addEventListener("keyup", () => updatePos(handle));
  handle.view.dom.addEventListener("mouseup", () => updatePos(handle));
}

function updatePos(handle: EditorHandle): void {
  const st = handle.view.state.selection.main.head;
  const line = handle.view.state.doc.lineAt(st).number;
  const col = st - handle.view.state.doc.line(line).from + 1;
  status.setPos(line, col, docStates.get(handle.docId)?.totalLines ?? line);
}

async function saveActive(): Promise<void> {
  const a = tabs.active();
  if (!a || busy) return;
  const ds = docStates.get(a.info.docId);
  if (!ds) return;
  setBusy(true);
  try {
    const report = await docSave({
      docId: a.info.docId,
      text: a.handle.getText(),
      path: ds.path,
      encoding: ds.encoding,
      eol: ds.eol,
      force: false,
    });
    afterSave(a.info.docId, a.handle, ds, report.replaceCount, report.conflict, false);
  } catch (err) {
    if (
      err instanceof IpcError &&
      (err.payload.code === "conflict" || err.payload.code === "unmappable")
    ) {
      // Cross-platform async dialog (I4): window.confirm is unreliable on
      // WKWebView / webkit2gtk.
      setBusy(false);
      const go = await ask(`${err.zh}\n\n确定要强制覆盖吗？`, {
        title: err.payload.code === "conflict" ? "保存冲突" : "编码无法表示",
        kind: "warning",
        okLabel: "强制覆盖",
        cancelLabel: "取消",
      });
      if (go) {
        setBusy(true);
        try {
          const report = await docSave({
            docId: a.info.docId,
            text: a.handle.getText(),
            path: ds.path,
            encoding: ds.encoding,
            eol: ds.eol,
            force: true,
          });
          afterSave(a.info.docId, a.handle, ds, report.replaceCount, report.conflict, true);
        } catch (err2) {
          reportError(err2);
        } finally {
          setBusy(false);
        }
      }
      return;
    }
    reportError(err);
  } finally {
    setBusy(false);
  }
}

function afterSave(
  docId: number,
  handle: EditorHandle,
  ds: DocState,
  replaceCount: number,
  conflict: boolean,
  forced: boolean,
): void {
  handle.markSaved();
  tabs.refreshDirty(docId);
  const total = handle.view.state.doc.lines; // line count after edits (Nit3)
  ds.totalLines = total;
  if (replaceCount > 0) {
    toastMsg(`已保存，但 ${replaceCount} 个字符在当前编码中写为占位符 ?`);
  } else if (conflict && forced) {
    toastMsg("检测到外部修改，已按你的选择强制覆盖");
  } else {
    updatePos(handle);
  }
}

/**
 * Switch the active document's encoding for real (C2): re-declare the label on
 * the shell (`doc_set_encoding`, which clears the cache + re-reads the head),
 * then re-fetch the whole text decoded under the new label and re-render. The
 * saved label always equals what the text was actually decoded as — never a
 * relabel that would corrupt a later `doc_save`.
 */
async function changeEncoding(label: string): Promise<void> {
  const a = tabs.active();
  if (!a || busy) return;
  const ds = docStates.get(a.info.docId);
  if (!ds || ds.encoding === label) return;
  if (a.handle.isDirty()) {
    const keep = await ask("切换编码会丢弃未保存的改动，是否继续？", {
      title: "未保存改动",
      kind: "warning",
      okLabel: "继续",
      cancelLabel: "取消",
    });
    if (!keep) {
      status.setEncoding(ds.encoding); // revert the dropdown selection
      return;
    }
  }
  setBusy(true);
  try {
    const updated = await docSetEncoding(a.info.docId, label, ds.totalLines, () => {});
    const { text } = await loadAllText(a.info.docId, ds.totalLines);
    a.handle.replaceText(text);
    ds.encoding = updated.encoding;
    ds.eol = updated.eol || ds.eol;
    status.setEncoding(updated.encoding);
    tabs.refreshDirty(a.info.docId);
  } catch (err) {
    reportError(err);
    status.setEncoding(ds.encoding);
  } finally {
    setBusy(false);
  }
}

function applyMode(docId: number, handle: EditorHandle, mode: ModeChoice): void {
  const ds = docStates.get(docId);
  if (!ds) return;
  const ext = mode === "auto" ? extOf(ds.path) : mode === "plain" ? "" : mode;
  handle.setMode(ext); // reconfigure language only — keeps undo + scroll (I2)
}

async function handleClose(docId: number): Promise<void> {
  const handle = tabs.get(docId);
  if (!handle) return;
  if (handle.isDirty()) {
    const go = await ask("文档有未保存改动，确定关闭？", {
      title: "关闭文档",
      kind: "warning",
      okLabel: "丢弃并关闭",
      cancelLabel: "取消",
    });
    if (!go) return;
  }
  tabs.remove(docId);
  docStates.delete(docId);
  void docClose(docId).catch(() => {}); // release the shell-side doc (I5)
}

function syncStatus(docId: number): void {
  const ds = docStates.get(docId);
  if (!ds) return;
  status.setEncoding(ds.encoding);
  status.setEol(ds.eol);
  const handle = tabs.get(docId);
  if (handle) {
    status.setDirty(handle.isDirty());
    updatePos(handle);
  }
}

// ---- Search (spec §6.2 read-only hit list; Ctrl+Shift+F). Task 9 scope is
// `search_run` + hit list + snippet + click-to-locate (plan: "仅展示命中列表与
// snippet"). A live cancel needs the async search (Task 10) — `search_run` is a
// synchronous command that runs to completion, so no dead cancel control is
// shown here. See CHANGELOG Ruling, consistent with Task 8 F2.

const SEARCH_RENDER_CAP = 1000; // rows actually kept in the DOM
let searchRendered = 0; // rows appended so far
let searchTotal = 0; // hits seen so far (a counter, never a retained array)
let searchBusy = false;

function openSearchFlow(): void {
  const a = tabs.active();
  if (!a) {
    toastMsg("请先打开一个文档");
    return;
  }
  renderSearchShell(a.info.docId);
}

function renderSearchShell(docId: number): void {
  const panel = document.getElementById("search-panel");
  if (!panel) return;
  panel.style.display = "";
  panel.textContent = "";

  const bar = document.createElement("div");
  bar.className = "search-head";
  // Inline input, not `window.prompt` (unreliable on WKWebView / webkit2gtk —
  // the same reason I4 dropped `window.confirm`).
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = "搜索内容（按当前编码的字面量匹配）";
  const go = document.createElement("button");
  go.textContent = "搜索";
  const statusLine = document.createElement("span");
  statusLine.className = "search-status";
  const closeBtn = document.createElement("button");
  closeBtn.textContent = "关闭";
  closeBtn.addEventListener("click", hideSearchPanel);
  const list = document.createElement("div");
  list.className = "search-list";

  const submit = () => {
    const pattern = input.value;
    if (pattern) void runSearch(docId, pattern, statusLine, list);
  };
  go.addEventListener("click", submit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") submit();
  });

  bar.append(input, go, statusLine, closeBtn);
  panel.append(bar, list);
  input.focus();
}

async function runSearch(
  docId: number,
  pattern: string,
  statusLine: HTMLElement,
  list: HTMLElement,
): Promise<void> {
  if (searchBusy || !docStates.get(docId)) return;
  searchBusy = true;
  searchRendered = 0;
  searchTotal = 0;
  list.textContent = "";
  statusLine.textContent = "搜索中…";
  try {
    await searchRun({
      docId,
      pattern,
      mode: "literal",
      caseSensitive: true,
      onBatch: (m) => appendHits(list, m, statusLine),
      onDone: (s) => {
        statusLine.textContent = `命中 ${s.matches} 处，扫描 ${s.scannedBytes} 字节`;
      },
    });
  } catch (err) {
    reportError(err);
    statusLine.textContent = "搜索失败";
  } finally {
    searchBusy = false;
  }
}

/** Append only new rows, bounded to `SEARCH_RENDER_CAP`; never retain hits. */
function appendHits(list: HTMLElement, batch: MatchJson[], statusLine: HTMLElement): void {
  searchTotal += batch.length;
  for (const h of batch) {
    if (searchRendered >= SEARCH_RENDER_CAP) break;
    const row = document.createElement("div");
    row.className = "search-row";
    row.textContent = `第 ${h.line + 1} 行：${h.snippet}`;
    row.addEventListener("click", () => jumpToLine(h.line));
    list.append(row);
    searchRendered += 1;
  }
  statusLine.textContent =
    searchTotal > SEARCH_RENDER_CAP
      ? `搜索中… 已发现 ${searchTotal} 处（仅显示前 ${SEARCH_RENDER_CAP}）`
      : `搜索中… 已发现 ${searchTotal} 处`;
}

function hideSearchPanel(): void {
  const panel = document.getElementById("search-panel");
  if (panel) panel.style.display = "none";
}

function jumpToLine(zeroBasedLine: number): void {
  const a = tabs.active();
  if (!a) return;
  const st = a.handle.view.state;
  const line = Math.min(Math.max(1, zeroBasedLine + 1), st.doc.lines);
  const pos = st.doc.line(line).from;
  a.handle.view.dispatch({
    selection: { anchor: pos },
    scrollIntoView: true,
  });
  a.handle.view.focus();
}

function reportError(err: unknown): void {
  const msg = err instanceof IpcError ? err.zh : String(err);
  toastMsg(msg);
  console.error(err);
}

function toastMsg(msg: string): void {
  const toast = document.getElementById("toast");
  if (!toast) return;
  toast.textContent = msg;
  toast.classList.add("visible");
  window.setTimeout(() => toast.classList.remove("visible"), 4000);
}

function setBusy(b: boolean): void {
  busy = b;
  status.setBusy(b);
}

function registerShortcuts(): void {
  window.addEventListener("keydown", (e) => {
    const mod = e.ctrlKey || e.metaKey;
    if (!mod) return;
    const key = e.key.toLowerCase();
    if (key === "s") {
      e.preventDefault();
      void saveActive();
    } else if (key === "o") {
      e.preventDefault();
      void openFile();
    } else if (key === "f" && e.shiftKey) {
      e.preventDefault();
      openSearchFlow();
    }
  });
}

layout();
registerShortcuts();
