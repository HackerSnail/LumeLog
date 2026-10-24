//! App wiring: layout, open/save/encoding flows over the Task 8 IPC. Heavy UI
//! behaviors (conflict / unmappable three-choice, external-change watch, crash
//! restore) are spec'd for later milestones; here we stand up the edit-mode
//! path (≤ EDIT_THRESHOLD) and route oversized files to a placeholder until the
//! Task 10 virtual list exists. GUI acceptance is the Task 11 `tauri dev` pass.

import { open } from "@tauri-apps/plugin-dialog";
import { docOpen, docSave, IpcError, type DocInfo } from "./ipc";
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
  onActivate: (id) => syncStatus(id),
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
      a.handle.setEol(eol);
      docStates.get(a.info.docId)!.eol = eol;
      tabs.refreshDirty(a.info.docId);
    }
  },
  onMode: (mode) => {
    const a = tabs.active();
    if (a) applyMode(a, mode);
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
  searchBtn.addEventListener("click", () => tabs.active()?.handle.openSearch());
  bar.append(openBtn, searchBtn);
  const toast = document.createElement("div");
  toast.id = "toast";
  toast.className = "toast";
  root.append(bar, tabsEl, statusEl, toast);
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
      // Large-file read-only mode is the Task 10 virtual list.
      reportError(
        new Error("该文件超过 200MB，只读大文件视图将在后续版本提供（当前仅支持编辑模式）"),
      );
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
    filename: extOf(path),
    eol: "LF",
  });
  docStates.set(info.docId, {
    path,
    encoding: info.encoding,
    eol: "LF",
    totalLines: total,
  });
  tabs.add({ docId: info.docId, path }, handle);
  trackCursor(handle);
  syncStatus(info.docId);
}

function trackCursor(handle: EditorHandle): void {
  // Light selection listeners keep the status-bar line/col in sync; the full
  // CodeMirror selection extension is wired in the Task 11 GUI polish.
  handle.view.dom.addEventListener("keyup", () => updatePos(handle));
  handle.view.dom.addEventListener("mouseup", () => updatePos(handle));
}

function updatePos(handle: EditorHandle): void {
  const st = handle.view.state.selection.main.head;
  const line = handle.view.state.doc.lineAt(st).number;
  const col = st - handle.view.state.doc.line(line).from + 1;
  status.setPos(line, col, docStates.get(handle.docId)?.totalLines ?? line);
}

async function saveActive(force: boolean): Promise<void> {
  const a = tabs.active();
  if (!a || busy) return;
  const ds = docStates.get(a.info.docId);
  if (!ds) return;
  setBusy(true);
  try {
    await docSave({
      docId: a.info.docId,
      text: a.handle.getText(),
      path: ds.path,
      encoding: ds.encoding,
      eol: ds.eol,
      force,
    });
    a.handle.markSaved();
    tabs.refreshDirty(a.info.docId);
  } catch (err) {
    if (err instanceof IpcError && (err.payload.code === "conflict" || err.payload.code === "unmappable")) {
      if (confirm(`${err.zh}\n\n确定要强制覆盖吗？`)) {
        setBusy(false);
        await saveActive(true);
        return;
      }
    } else {
      reportError(err);
    }
  } finally {
    setBusy(false);
  }
}

async function changeEncoding(label: string): Promise<void> {
  const a = tabs.active();
  if (!a || busy) return;
  const ds = docStates.get(a.info.docId);
  if (!ds || ds.encoding === label) return;
  setBusy(true);
  try {
    // Re-declare the label and re-render the full text (spec §8). The shell
    // re-detects on `doc_open`; a dedicated set-encoding IPC is added with the
    // Task 11 integration pass. Until then we re-open to pick up the re-decode.
    const info = await docOpen(ds.path, () => {});
    docStates.delete(a.info.docId);
    tabs.remove(a.info.docId);
    await mountDoc({ ...info, encoding: label }, ds.path);
    const fresh = tabs.active();
    if (fresh) {
      docStates.get(fresh.info.docId)!.encoding = label;
      status.setEncoding(label);
    }
  } catch (err) {
    reportError(err);
  } finally {
    setBusy(false);
  }
}

function applyMode(a: { info: { docId: number }; handle: EditorHandle }, mode: ModeChoice): void {
  const ds = docStates.get(a.info.docId);
  if (!ds) return;
  const ext = mode === "auto" ? extOf(ds.path) : mode === "plain" ? "" : mode;
  a.handle.setMode(ext);
}

async function handleClose(docId: number): Promise<void> {
  const handle = tabs.get(docId);
  if (!handle) return;
  if (handle.isDirty() && !confirm("文档有未保存改动，确定关闭？")) return;
  tabs.remove(docId);
  docStates.delete(docId);
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

function reportError(err: unknown): void {
  const msg = err instanceof IpcError ? err.zh : String(err);
  const toast = document.getElementById("toast");
  if (toast) {
    toast.textContent = msg;
    toast.classList.add("visible");
    window.setTimeout(() => toast.classList.remove("visible"), 4000);
  }
  console.error(err);
}

function setBusy(b: boolean): void {
  busy = b;
  status.setBusy(b);
}

function registerShortcuts(): void {
  window.addEventListener("keydown", (e) => {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void saveActive(false);
    } else if (mod && e.key.toLowerCase() === "o") {
      e.preventDefault();
      void openFile();
    }
  });
}

layout();
registerShortcuts();
