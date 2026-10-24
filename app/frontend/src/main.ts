//! App wiring: layout, open/save/encoding/search flows over the Task 8 IPC.
//! `doc_open` returns `mode` (spec §4.3/A1): `"edit"` (≤200MB) mounts CodeMirror,
//! `"readonly"` (>200MB) mounts the virtual list (Task 10). GUI acceptance
//! (`tauri dev`, GBK no-mojibake roundtrip, drag-drop, recent-20, 1GB list
//! scroll) is the Task 11 pass; the logic below is headless-checked by
//! `tsc --noEmit` (strict) + `vitest`.

import { open, save, ask } from "@tauri-apps/plugin-dialog";
import {
  docOpen,
  docClose,
  docSave,
  docSetEncoding,
  loadAllText,
  loadWindow,
  searchRun,
  appPing,
  IpcError,
  type DocInfo,
  type IndexProgress,
  type MatchJson,
} from "./ipc";
import { openEditable, type EditorHandle } from "./editor";
import { ListView } from "./listview";
import { TabStrip } from "./tabs";
import { Statusbar } from "./statusbar";
import { extOf } from "./encoding-menu";
import type { ModeChoice } from "./language";
import "./styles.css";

interface DocState {
  path: string;
  encoding: string;
  eol: string;
  totalLines: number;
  mode: "edit" | "readonly";
  /** Unmappable bytes in the last decoded window (spec §5.4 persistent ⚠). */
  invalid: number;
}

const docStates = new Map<number, DocState>();
/** Rich, kind-specific handles keyed by doc id (the tab strip holds the views). */
const editors = new Map<number, EditorHandle>();
const lists = new Map<number, ListView>();
/** Lines indexed so far per doc; drives the list "待索引" placeholder (Task 11). */
const indexed = new Map<number, number>();
let busy = false;

const tabsEl = document.createElement("div");
tabsEl.id = "tabs";
const statusEl = document.createElement("div");
statusEl.id = "status";

const tabs = new TabStrip(tabsEl, {
  onActivate: (id) => {
    hideSearchPanel(); // stale hit rows belong to the previous doc — drop the panel
    if (id < 0) status.clear();
    else syncStatus(id);
  },
  onClose: (id) => void handleClose(id),
  onDirtyChanged: (id, dirty) => {
    if (id === tabs.active()?.info.docId) status.setDirty(dirty);
  },
});
const status = new Statusbar(statusEl, {
  onEncoding: (label) => void changeEncoding(label),
  onEol: (eol) => {
    const a = tabs.active();
    const handle = a && editors.get(a.info.docId);
    if (a && handle) {
      handle.setEol(eol.toUpperCase());
      docStates.get(a.info.docId)!.eol = eol.toUpperCase();
      tabs.refreshDirty(a.info.docId);
    }
  },
  onMode: (mode) => {
    const a = tabs.active();
    if (a) applyMode(a.info.docId, mode);
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
    const info = await docOpen(path, onIndexProgress);
    // The shell owns the ≤200MB split (spec §4.3/A1) via `DocInfo.mode`.
    if (info.mode === "readonly") mountList(info, path);
    else await mountDoc(info, path);
  } catch (err) {
    reportError(err);
  } finally {
    setBusy(false);
  }
}

/** Index progress tick. Sync indexing (v1) emits one `done`; Task 11 streams it. */
function onIndexProgress(p: IndexProgress): void {
  if (!p.done) return;
  const ds = docStates.get(p.docId);
  if (ds) indexed.set(p.docId, ds.totalLines);
  lists.get(p.docId)?.refresh();
}

async function mountDoc(info: DocInfo, path: string): Promise<void> {
  const total = info.totalLines ?? info.lineCountEstimate;
  indexed.set(info.docId, total);
  docStates.set(info.docId, {
    path,
    encoding: info.encoding,
    eol: info.eol || "LF",
    totalLines: total,
    mode: "edit",
    invalid: 0,
  });
  const handle = await openEditable({
    parent: tabsEl,
    info,
    ext: extOf(path),
    onDirty: () => tabs.refreshDirty(info.docId),
    onInvalid: (n) => {
      // Persistent ⚠ beside the encoding (spec §5.4), not a transient toast —
      // a wrong-encoding file must stay discoverable while it is open.
      const d = docStates.get(info.docId);
      if (d) d.invalid = n;
      status.setInvalidCount(n);
    },
  });
  editors.set(info.docId, handle);
  tabs.add({ docId: info.docId, path }, handle);
  trackCursor(handle);
  syncStatus(info.docId);
}

/** Mount the read-only virtual list (spec §4.3). No full load — windows on demand. */
function mountList(info: DocInfo, path: string): void {
  const total = info.totalLines ?? info.lineCountEstimate;
  indexed.set(info.docId, total);
  docStates.set(info.docId, {
    path,
    encoding: info.encoding,
    eol: info.eol || "LF",
    totalLines: total,
    mode: "readonly",
    invalid: 0,
  });
  const list = new ListView({
    docId: info.docId,
    totalLines: () => docStates.get(info.docId)?.totalLines ?? 0,
    indexedLines: () => indexed.get(info.docId) ?? docStates.get(info.docId)?.totalLines ?? 0,
    loadWindow: (start, count) => loadWindow(info.docId, start, count),
    onInvalidCount: (n) => {
      // §5.4: every fetched window reports its unmappable count; keep the
      // status-bar ⚠ in sync (only repaint if this doc is the shown one).
      const d = docStates.get(info.docId);
      if (d) d.invalid = n;
      if (info.docId === tabs.active()?.info.docId) status.setInvalidCount(n);
    },
  });
  lists.set(info.docId, list);
  tabs.add({ docId: info.docId, path }, list);
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

async function saveActive(saveAs = false): Promise<void> {
  const a = tabs.active();
  if (!a || busy) return;
  const handle = editors.get(a.info.docId);
  if (!handle) {
    toastMsg("只读视图不支持编辑或保存");
    return;
  }
  const ds = docStates.get(a.info.docId);
  if (!ds) return;

  // Save-As (Ctrl+Shift+S): pick a destination first; cancelling aborts quietly.
  let target = ds.path;
  if (saveAs) {
    const dest = await save({
      defaultPath: ds.path,
      filters: [{ name: "日志/文本", extensions: ["*"] }],
    });
    if (typeof dest !== "string") return;
    target = dest;
  }

  setBusy(true);
  try {
    const report = await docSave({
      docId: a.info.docId,
      text: handle.getText(),
      path: target,
      encoding: ds.encoding,
      eol: ds.eol,
      force: false,
    });
    afterSave(a.info.docId, handle, ds, report.replaceCount, report.conflict, false, target);
  } catch (err) {
    if (
      err instanceof IpcError &&
      (err.payload.code === "conflict" || err.payload.code === "unmappable")
    ) {
      // Cross-platform async dialog (I4): window.confirm is unreliable on
      // WKWebView / webkit2gtk. One *informed* confirmation folds both gates
      // (final-review I-1): the shell's conflict detail already spells out the
      // lossy-`?` consequence, so a single force prompt discloses everything.
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
            text: handle.getText(),
            path: target,
            encoding: ds.encoding,
            eol: ds.eol,
            force: true,
          });
          afterSave(a.info.docId, handle, ds, report.replaceCount, report.conflict, true, target);
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
  target: string,
): void {
  ds.path = target; // a Save-As rebinds the tab to its new file
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
  const docId = a.info.docId;
  const ds = docStates.get(docId);
  if (!ds || ds.encoding === label) return;
  const handle = editors.get(docId); // undefined for a read-only doc
  const list = lists.get(docId);
  if (handle && handle.isDirty()) {
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
    // Re-decode shell-side (clears the chunk cache); the returned encoding is truth.
    const updated = await docSetEncoding(docId, label, ds.totalLines, () => {});
    if (handle) {
      const { text, invalidCount } = await loadAllText(docId, ds.totalLines);
      handle.replaceText(text);
      // Refresh the persistent ⚠ for the NEW encoding (spec §5.4): the whole
      // point is a wrong-encoding file stays discoverable *right after* a
      // switch. The read-only path does the same via invalidate→refetch→
      // onInvalidCount; the edit path loads here, so update it here too.
      ds.invalid = invalidCount;
      status.setInvalidCount(invalidCount);
    } else if (list) {
      list.invalidate(); // drop stale-decoded windows; refetch on next paint
    }
    ds.encoding = updated.encoding;
    ds.eol = updated.eol || ds.eol;
    status.setEncoding(updated.encoding);
    if (handle) tabs.refreshDirty(docId);
  } catch (err) {
    reportError(err);
    status.setEncoding(ds.encoding);
  } finally {
    setBusy(false);
  }
}

function applyMode(docId: number, mode: ModeChoice): void {
  const ds = docStates.get(docId);
  const handle = editors.get(docId); // read-only docs have no syntax mode
  if (!ds || !handle) return;
  const ext = mode === "auto" ? extOf(ds.path) : mode === "plain" ? "" : mode;
  handle.setMode(ext); // reconfigure language only — keeps undo + scroll (I2)
}

async function handleClose(docId: number): Promise<void> {
  const view = tabs.get(docId);
  if (!view) return;
  if (view.isDirty()) {
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
  editors.delete(docId);
  lists.delete(docId);
  indexed.delete(docId);
  void docClose(docId).catch(() => {}); // release the shell-side doc (I5)
}

function syncStatus(docId: number): void {
  const ds = docStates.get(docId);
  if (!ds) return;
  status.setEncoding(ds.encoding);
  status.setEol(ds.eol);
  status.setInvalidCount(ds.invalid); // §5.4 — per-doc, restored on tab switch
  const view = tabs.get(docId);
  if (view) status.setDirty(view.isDirty());
  const handle = editors.get(docId); // line/col only for editable docs
  if (handle) updatePos(handle);
  else status.setReadonly(ds.totalLines);
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
  if (searchBusy) {
    toastMsg("搜索进行中，请稍候…");
    return;
  }
  renderSearchShell();
}

function renderSearchShell(): void {
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

  // Re-resolve the active doc on every submit: the panel persists across tab
  // switches, so a frozen docId would scan the wrong document (round-2 fix).
  const submit = () => {
    const pattern = input.value;
    const a = tabs.active();
    if (!pattern || !a) return;
    if (searchBusy) {
      statusLine.textContent = "搜索进行中，请稍候…";
      return;
    }
    void runSearch(a.info.docId, pattern, statusLine, list);
  };
  go.addEventListener("click", submit);
  input.addEventListener("keydown", (e) => {
    // Skip Enter while an IME is composing, else confirming a Chinese candidate
    // word fires a search on a half-typed pattern.
    if (e.key === "Enter" && !e.isComposing) submit();
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
  if (searchBusy) return;
  if (!docStates.get(docId)) {
    statusLine.textContent = "文档已关闭";
    return;
  }
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
        statusLine.textContent =
          `命中 ${s.matches} 处，扫描 ${s.scannedBytes} 字节` +
          (s.matches > SEARCH_RENDER_CAP ? `（仅显示前 ${SEARCH_RENDER_CAP}）` : "");
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
  const handle = editors.get(a.info.docId);
  if (handle) {
    const st = handle.view.state;
    const line = Math.min(Math.max(1, zeroBasedLine + 1), st.doc.lines);
    const pos = st.doc.line(line).from;
    handle.view.dispatch({ selection: { anchor: pos }, scrollIntoView: true });
    handle.view.focus();
    return;
  }
  lists.get(a.info.docId)?.scrollToLine(zeroBasedLine); // read-only list
}

/** Ctrl+G: a small inline go-to-line box (not `window.prompt` — unreliable on
 *  WKWebView / webkit2gtk, the same reason the search panel uses an `<input>`). */
function openJumpPrompt(): void {
  if (!tabs.active()) {
    toastMsg("请先打开一个文档");
    return;
  }
  const existing = document.getElementById("jump-panel");
  if (existing) {
    (existing.querySelector("input") as HTMLInputElement | null)?.focus();
    return;
  }
  const panel = document.createElement("div");
  panel.id = "jump-panel";
  panel.className = "jump-panel";
  const input = document.createElement("input");
  input.type = "text";
  input.inputMode = "numeric";
  input.placeholder = "跳转到行号";
  const go = document.createElement("button");
  go.textContent = "跳转";
  const close = () => panel.remove();
  const submit = () => {
    const n = Number.parseInt(input.value, 10);
    if (Number.isFinite(n) && n >= 1) {
      jumpToLine(n - 1); // jumpToLine takes a 0-based line
      close();
    } else {
      toastMsg("请输入有效行号（≥1）");
    }
  };
  go.addEventListener("click", submit);
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) submit();
    else if (e.key === "Escape") close();
  });
  panel.append(input, go);
  document.getElementById("root")?.append(panel);
  input.focus();
  input.select();
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
      void saveActive(e.shiftKey); // Ctrl+S save, Ctrl+Shift+S save-as (spec §12 M1)
    } else if (key === "o") {
      e.preventDefault();
      void openFile();
    } else if (key === "g") {
      e.preventDefault();
      openJumpPrompt(); // Ctrl+G jump-to-line (spec §8)
    } else if (key === "f" && e.shiftKey) {
      e.preventDefault();
      openSearchFlow();
    }
  });
}

layout();
registerShortcuts();
// First-interaction probe (spec P1/P2): the app_ping round-trip is the wired
// timing endpoint for the manual GUI cold-start measurement. A headless CI
// cannot observe real first-paint, so §2 records P1/P2 as human smoke items.
void appPing().catch(() => {});
