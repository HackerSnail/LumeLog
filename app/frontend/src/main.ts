//! App wiring: layout, open/save/encoding/search flows over the Task 8 IPC.
//! `doc_open` returns `mode` (spec §4.3/A1): `"edit"` (≤200MB) mounts CodeMirror,
//! `"readonly"` (>200MB) mounts the virtual list (Task 10). GUI acceptance
//! (`tauri dev`, GBK no-mojibake roundtrip, drag-drop, recent-20, 1GB list
//! scroll) is the Task 11 pass; the logic below is headless-checked by
//! `tsc --noEmit` (strict) + `vitest`.

import { open, save, ask } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  docOpen,
  docClose,
  docSave,
  docSetEncoding,
  loadAllText,
  loadWindow,
  appPing,
  dbcImport,
  canParse,
  canCancel,
  canDecodeSnippet,
  filterRun,
  filterCancel,
  docLinesAt,
  exportLines,
  exportCancel,
  exportCsv,
  exportJsonl,
  exportCanCancel,
  recoverySave,
  recoveryDiscard,
  recoveryList,
  recoveryLoad,
  recoveryEnabled,
  recoverySetEnabled,
  IpcError,
  type DocInfo,
  type IndexProgress,
  type FrameJson,
  type DbcInfo,
  type FilterCondition,
  type LineHit,
  type IndexedText,
  type TimeFormat,
  type CanExportStats,
  type RecoveryEntry,
} from "./ipc";
import { pickTimelineInput, type TimelineSelection } from "./query";
import { Debouncer, formatSavedAt, humanSize } from "./recovery";
import { openEditable, type EditorHandle } from "./editor";
import { ListView } from "./listview";
import { CanPanel, FrameStore } from "./canpanel";
import { type CanFilter } from "./canfilter";
import { TabStrip } from "./tabs";
import { Statusbar } from "./statusbar";
import { extOf } from "./encoding-menu";
import type { ModeChoice } from "./language";
import { initTheme } from "./theme";
import {
  makeResizable,
  loadFlag,
  storeFlag,
  MIN_SIDEBAR,
  MAX_SIDEBAR,
  MIN_INSPECTOR,
  MAX_INSPECTOR,
} from "./shell";
import { filterCommands, type Command } from "./palette";
import "./styles.css";

// Paint the stored theme as early as module eval allows (index.html also runs a
// pre-paint inline script to dodge a light flash on a dark preference).
const theme = initTheme();

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
/** Crash-recovery schedule (spec §4.4): 2s idle → write the doc's shadow. */
let recoveryOn = true; // refreshed from config.toml at startup (Task 6)
const recoveryDebounce = new Debouncer(2000, (docId) => fireRecovery(docId));
/** In-flight shadow writes per path: discard must wait, or a late write revives it. */
const recoveryWrites = new Map<string, Promise<void>>();
let recoveryChk: HTMLInputElement | null = null;
let recoveryBannerEl: HTMLElement | null = null;
let recoveryEntries: RecoveryEntry[] = [];

// ---- CanPanel / CAN decode state (spec §7.4) --------------------------------
// One panel instance is re-attached to whichever document is active; each
// document keeps its own accumulated frames + last filter so switching tabs
// restores that doc's table instead of the previous one's.
const canStores = new Map<number, FrameStore>();
const canFilters = new Map<number, CanFilter>();
let lastDbcId: number | null = null; // most-recently imported DBC, reused per parse
let canPanel: CanPanel | null = null; // lazily built on first dock open
let canDockEl: HTMLElement | null = null;
let canDockVisible = false;
let canParsing = false;
let canReqId: number | null = null; // job id from the (now early) can_parse resolve, for Stop
// Dock controls, resolved in layout(); guarded null until then.
let canSrcSel: HTMLSelectElement | null = null;
let canRegexInput: HTMLInputElement | null = null;
let canRegexRow: HTMLElement | null = null;
let canDbcLine: HTMLElement | null = null;
// CAN export (spec §7.5): shares the dock's single-flight with parsing.
let canExportBusy = false;
let canExportReqId: number | null = null;
let canExportStatus: HTMLElement | null = null;
let canExportCancelBtn: HTMLButtonElement | null = null;
let canExportBtns: HTMLButtonElement[] = [];
let canRunBtn: HTMLButtonElement | null = null;
let canHostEl: HTMLElement | null = null;
// filter inputs (per-active-doc view filter)
const canF = {} as Record<string, HTMLInputElement | HTMLSelectElement>;

const tabsEl = document.createElement("div");
tabsEl.id = "tabs";
const statusEl = document.createElement("div");
statusEl.id = "status";

// Shell panels, populated by layout(); the update helpers guard on null so
// they are safe before layout() runs.
let sidebarEl: HTMLElement | null = null;
let sidebarListEl: HTMLElement | null = null;
let sidebarHandleEl: HTMLElement | null = null;
let inspectorEl: HTMLElement | null = null;
let inspectorBodyEl: HTMLElement | null = null;
let inspectorHandleEl: HTMLElement | null = null;
let sidebarCollapsed = false;
let inspectorCollapsed = false;

const tabs = new TabStrip(tabsEl, {
  onActivate: (id) => {
    hideSearchPanel(); // stale hit rows belong to the previous doc — drop the panel
    if (id < 0) status.clear();
    else syncStatus(id);
    refreshPanels();
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
  const root = document.getElementById("app");
  if (!root) return;
  root.textContent = "";

  const bar = document.createElement("div");
  bar.className = "appbar";
  // Toolbar — with `decorations:false` this doubles as the window titlebar:
  // app name + actions on the left, a draggable filler, window buttons on the
  // right (buttons are only revealed when running under Tauri — see detectChrome).
  const appName = document.createElement("span");
  appName.className = "app-name";
  appName.textContent = "LumeLog";
  bar.append(
    appName,
    mkBtn("打开文件", "", () => void openFile()),
    mkBtn("查询", "行过滤 / 高亮命中 (Ctrl+Shift+F)", () => openQueryPanel()),
    mkBtn("报文面板", "显示 / 隐藏 CAN 报文面板", () => toggleCanDock()),
    mkBtn("侧栏", "显示 / 隐藏侧栏", () => toggleSidebar()),
    mkBtn("信息", "显示 / 隐藏文档信息栏", () => toggleInspector()),
    mkBtn("主题", "切换主题（浅色 / 暗黑 / 吸血鬼 / 跟随系统）", () => theme.cycle()),
  );

  const drag = document.createElement("div");
  drag.className = "titlebar-drag";
  drag.setAttribute("data-tauri-drag-region", "");

  const winBtns = document.createElement("div");
  winBtns.className = "win-btns";
  winBtns.append(
    mkWinBtn("﹣", "最小化", () => winAction("min")),
    mkWinBtn("▢", "最大化 / 还原", () => winAction("max")),
    mkWinBtn("✕", "关闭", () => winAction("close"), "close"),
  );
  bar.append(drag, winBtns);

  // Left sidebar: the set of open documents (live from the tab strip).
  const aside = document.createElement("aside");
  aside.className = "sidebar";
  sidebarEl = aside;
  const asideTitle = document.createElement("div");
  asideTitle.className = "panel-title";
  asideTitle.textContent = "已打开文档";
  sidebarListEl = document.createElement("div");
  sidebarListEl.className = "side-list";
  const recoveryOption = document.createElement("label");
  recoveryOption.className = "side-option";
  recoveryChk = document.createElement("input");
  recoveryChk.type = "checkbox";
  recoveryChk.id = "recovery-enabled";
  recoveryChk.checked = true;
  recoveryChk.addEventListener("change", () => void onRecoveryToggle(recoveryChk!.checked));
  recoveryOption.append(recoveryChk, document.createTextNode("崩溃恢复"));
  aside.append(asideTitle, sidebarListEl, recoveryOption);

  // Right inspector: metadata for the active document + the ⚠ detail.
  const insp = document.createElement("aside");
  insp.className = "inspector";
  inspectorEl = insp;
  const inspTitle = document.createElement("div");
  inspTitle.className = "panel-title";
  inspTitle.textContent = "文档信息";
  inspectorBodyEl = document.createElement("div");
  inspectorBodyEl.className = "insp-body";
  insp.append(inspTitle, inspectorBodyEl);

  const center = document.createElement("div");
  center.className = "center";
  const panel = document.createElement("div");
  panel.id = "search-panel";
  panel.className = "search-panel";
  panel.style.display = "none";
  center.append(tabsEl, panel, buildCanDock());

  sidebarHandleEl = document.createElement("div");
  sidebarHandleEl.className = "pane-resizer";
  inspectorHandleEl = document.createElement("div");
  inspectorHandleEl.className = "pane-resizer";
  makeResizable(aside, sidebarHandleEl, {
    min: MIN_SIDEBAR,
    max: MAX_SIDEBAR,
    storageKey: "lumelog.ui.sidebar.w",
    edge: "left",
    collapsed: () => sidebarCollapsed,
  });
  makeResizable(insp, inspectorHandleEl, {
    min: MIN_INSPECTOR,
    max: MAX_INSPECTOR,
    storageKey: "lumelog.ui.inspector.w",
    edge: "right",
    collapsed: () => inspectorCollapsed,
  });

  const workspace = document.createElement("div");
  workspace.className = "workspace";
  workspace.append(aside, sidebarHandleEl, center, inspectorHandleEl, insp);

  const toast = document.createElement("div");
  toast.id = "toast";
  toast.className = "toast";

  sidebarCollapsed = loadFlag("lumelog.ui.sidebar", false);
  inspectorCollapsed = loadFlag("lumelog.ui.inspector", false);
  aside.classList.toggle("collapsed", sidebarCollapsed);
  insp.classList.toggle("collapsed", inspectorCollapsed);
  sidebarHandleEl.style.display = sidebarCollapsed ? "none" : "";
  inspectorHandleEl.style.display = inspectorCollapsed ? "none" : "";

  root.append(bar, workspace, statusEl, toast);
  refreshPanels();
}

function mkBtn(label: string, title: string, on: () => void): HTMLButtonElement {
  const b = document.createElement("button");
  b.textContent = label;
  if (title) b.title = title;
  b.addEventListener("click", on);
  return b;
}

function mkWinBtn(
  glyph: string,
  title: string,
  on: () => void,
  extra = "",
): HTMLButtonElement {
  const b = document.createElement("button");
  b.className = "win-btn" + (extra ? " " + extra : "");
  b.textContent = glyph;
  b.title = title;
  b.addEventListener("click", on);
  return b;
}

/** Drive the frameless window; guarded so a plain-browser `vite dev` (no Tauri
 *  host, `getCurrentWindow()` throws) degrades to a toast instead of a crash. */
function winAction(kind: "min" | "max" | "close"): void {
  try {
    const w = getCurrentWindow();
    void (kind === "min" ? w.minimize() : kind === "max" ? w.toggleMaximize() : w.close());
  } catch {
    toastMsg("窗口控制不可用（非 Tauri 环境）");
  }
}

/** Reveal the custom window buttons only under a real Tauri host, and tag the
 *  platform so CSS can tune the titlebar chrome. */
function detectChrome(): void {
  const el = document.documentElement;
  const inTauri =
    typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ ===
    "object";
  if (inTauri) el.classList.add("has-custom-titlebar");
  const ua = navigator.userAgent;
  const plat = ua.includes("Windows")
    ? "windows"
    : ua.includes("Mac")
      ? "mac"
      : ua.includes("Linux")
        ? "linux"
        : "";
  if (plat) el.dataset.tauriPlatform = plat;
}

function toggleSidebar(): void {
  sidebarCollapsed = !sidebarCollapsed;
  sidebarEl?.classList.toggle("collapsed", sidebarCollapsed);
  if (sidebarHandleEl) sidebarHandleEl.style.display = sidebarCollapsed ? "none" : "";
  storeFlag("lumelog.ui.sidebar", sidebarCollapsed);
}

function toggleInspector(): void {
  inspectorCollapsed = !inspectorCollapsed;
  inspectorEl?.classList.toggle("collapsed", inspectorCollapsed);
  if (inspectorHandleEl) inspectorHandleEl.style.display = inspectorCollapsed ? "none" : "";
  storeFlag("lumelog.ui.inspector", inspectorCollapsed);
}

function basenameOf(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

/** Rebuild the sidebar's open-document list from the live tab strip. */
function refreshSidebar(): void {
  const list = sidebarListEl;
  if (!list) return;
  list.textContent = "";
  const infos = tabs.list();
  const activeId = tabs.active()?.info.docId ?? null;
  if (infos.length === 0) {
    const empty = document.createElement("div");
    empty.className = "insp-empty";
    empty.style.padding = "8px 10px";
    empty.textContent = "尚无打开的文档";
    list.append(empty);
    return;
  }
  for (const info of infos) {
    const row = document.createElement("div");
    row.className = "side-item";
    row.title = info.path;
    if (info.docId === activeId) row.classList.add("active");
    const enc = docStates.get(info.docId)?.encoding;
    if (enc) {
      const m = document.createElement("span");
      m.className = "side-meta";
      m.textContent = enc;
      row.append(m);
    }
    const label = document.createElement("span");
    label.textContent = basenameOf(info.path);
    row.append(label);
    row.addEventListener("click", () => tabs.activate(info.docId));
    list.append(row);
  }
}

/** Fill the inspector with the active document's metadata (spec §5.4 ⚠ detail). */
function updateInspector(docId: number | null): void {
  const body = inspectorBodyEl;
  if (!body) return;
  body.textContent = "";
  const ds = docId != null ? docStates.get(docId) : undefined;
  if (!ds) {
    const empty = document.createElement("div");
    empty.className = "insp-empty";
    empty.textContent = "无打开文档";
    body.append(empty);
    return;
  }
  const rows: [string, string, boolean?][] = [
    ["模式", ds.mode === "edit" ? "编辑（可修改）" : "只读（大文件）"],
    ["编码", ds.encoding],
    ["行尾", ds.eol],
    ["总行数", String(ds.totalLines)],
    ["已索引", String(indexed.get(docId ?? 0) ?? ds.totalLines)],
    ["无法映射字符", String(ds.invalid), ds.invalid > 0],
    ["路径", ds.path],
  ];
  for (const [key, value, warn] of rows) {
    const row = document.createElement("div");
    row.className = "insp-row";
    const k = document.createElement("span");
    k.className = "insp-key";
    k.textContent = key;
    const v = document.createElement("span");
    v.className = warn ? "insp-val warn" : "insp-val";
    v.textContent = value;
    if (key === "路径") v.title = value;
    row.append(k, v);
    body.append(row);
  }
}

function refreshPanels(): void {
  refreshSidebar();
  updateInspector(tabs.active()?.info.docId ?? null);
  switchCanView();
}

// ---- CAN decode dock (spec §7.4) --------------------------------------------

/** Build (once) and return the message dock: source/DBC controls, a filter row,
 *  and the CanPanel host. Hidden until `toggleCanDock` reveals it. */
function buildCanDock(): HTMLElement {
  if (canDockEl) return canDockEl;
  const dock = document.createElement("div");
  dock.id = "can-dock";
  dock.className = "can-dock";
  dock.style.display = "none";

  const bar = document.createElement("div");
  bar.className = "can-dockbar";

  canSrcSel = document.createElement("select");
  canSrcSel.className = "can-src";
  for (const [v, t] of [
    ["candump", "candump"],
    ["asc", "ASC"],
    ["regex", "正则"],
  ]) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = t;
    canSrcSel.append(o);
  }
  canSrcSel.addEventListener("change", () => {
    if (canRegexRow) canRegexRow.style.display = canSrcSel!.value === "regex" ? "" : "none";
  });

  canRegexRow = document.createElement("span");
  canRegexRow.className = "can-regex-row";
  canRegexRow.style.display = "none";
  canRegexInput = document.createElement("input");
  canRegexInput.type = "text";
  canRegexInput.placeholder = "命名组 (?<id>...) 与 (?<data>...)";
  canRegexRow.append(document.createTextNode("正则 "), canRegexInput);

  const importBtn = mkBtn("导入 DBC", "导入 .dbc 信号数据库", () => void importDbc());
  canDbcLine = document.createElement("span");
  canDbcLine.className = "can-dbc-line";
  canDbcLine.textContent = "未导入 DBC";

  canRunBtn = mkBtn("解析报文", "解码当前文档为报文表格", () => void onCanRunClick());
  const decodeBtn = mkBtn("识别选中", "把编辑器选中文本识别为一条报文", () => void decodeSelection());
  const csvBtn = mkBtn("导出 CSV", "把当前过滤的报文导出为宽表 CSV", () => void onCanExport("csv"));
  const jsonlBtn = mkBtn("导出 JSONL", "把当前过滤的报文导出为 JSONL", () => void onCanExport("jsonl"));
  canExportBtns = [csvBtn, jsonlBtn];
  canExportCancelBtn = mkBtn("取消导出", "停止正在进行的导出", () => cancelCanExport());
  canExportCancelBtn.style.display = "none";
  canExportStatus = document.createElement("span");
  canExportStatus.className = "can-export-status";

  bar.append(
    document.createTextNode("源 "),
    canSrcSel,
    canRegexRow,
    importBtn,
    canDbcLine,
    canRunBtn,
    decodeBtn,
    csvBtn,
    jsonlBtn,
    canExportCancelBtn,
    canExportStatus,
  );

  dock.append(bar, buildCanFilterRow(), (canHostEl = mkHost()));
  canDockEl = dock;
  return dock;
}

/** The view-filter toolbar; each change rebuilds the active doc's `CanFilter`. */
function buildCanFilterRow(): HTMLElement {
  const row = document.createElement("div");
  row.className = "can-filter";
  const num = (key: string, ph: string): HTMLInputElement => {
    const i = document.createElement("input");
    i.type = "text";
    i.placeholder = ph;
    i.className = "can-fnum";
    canF[key] = i;
    i.addEventListener("change", () => applyFilterFromInputs());
    row.append(i);
    return i;
  };
  const txt = (key: string, ph: string): HTMLInputElement => {
    const i = num(key, ph);
    i.classList.add("can-ftxt");
    return i;
  };
  row.append(document.createTextNode("ID "));
  txt("idFrom", "起(hex/dec)");
  row.append(document.createTextNode("–"));
  txt("idTo", "止");
  row.append(document.createTextNode(" 消息 "));
  txt("msgName", "名");
  row.append(document.createTextNode(" 信号 "));
  txt("signalName", "名");
  const op = document.createElement("select");
  canF.signalOp = op;
  for (const o of [">", "<", ">=", "<=", "=="]) {
    const opt = document.createElement("option");
    opt.value = o;
    opt.textContent = o;
    op.append(opt);
  }
  op.addEventListener("change", () => applyFilterFromInputs());
  row.append(op);
  txt("signalValue", "值");
  row.append(document.createTextNode(" 时间(ms) "));
  num("timeFromMs", "起");
  row.append(document.createTextNode("–"));
  num("timeToMs", "止");
  const clear = mkBtn("清除过滤", "清空视图过滤条件", () => {
    for (const el of Object.values(canF)) el.value = "";
    // the operator is a select: reset it to its default rather than a blank `-1`
    if (canF.signalOp) (canF.signalOp as HTMLSelectElement).value = ">";
    applyFilterFromInputs();
  });
  row.append(clear);
  return row;
}

function mkHost(): HTMLElement {
  const host = document.createElement("div");
  host.className = "can-host";
  return host;
}

/** Parse a filter field: numeric, or hex `0x..`; empty → undefined. */
function parseNum(v: string): number | undefined {
  const s = v.trim();
  if (s === "") return undefined;
  const n = s.toLowerCase().startsWith("0x")
    ? Number.parseInt(s, 16)
    : Number(s);
  return Number.isFinite(n) ? n : undefined;
}

/** Rebuild a `CanFilter` from the input row and push it to the panel. */
function applyFilterFromInputs(): void {
  const a = tabs.active();
  if (!a) return;
  const f: CanFilter = {};
  const idFrom = parseNum(canF.idFrom?.value ?? "");
  const idTo = parseNum(canF.idTo?.value ?? "");
  const sigVal = parseNum(canF.signalValue?.value ?? "");
  const tFrom = parseNum(canF.timeFromMs?.value ?? "");
  const tTo = parseNum(canF.timeToMs?.value ?? "");
  if (idFrom !== undefined) f.idFrom = idFrom;
  if (idTo !== undefined) f.idTo = idTo;
  const msg = canF.msgName?.value.trim();
  if (msg) f.msgName = msg;
  const sig = canF.signalName?.value.trim();
  if (sig && sigVal !== undefined) {
    f.signalName = sig;
    f.signalOp = (canF.signalOp as HTMLSelectElement).value as CanFilter["signalOp"];
    f.signalValue = sigVal;
  }
  if (tFrom !== undefined) f.timeFromMs = tFrom;
  if (tTo !== undefined) f.timeToMs = tTo;
  canFilters.set(a.info.docId, f);
  canPanel?.setFilter(f);
}

function toggleCanDock(): void {
  canDockVisible = !canDockVisible;
  if (canDockEl) canDockEl.style.display = canDockVisible ? "" : "none";
  if (canDockVisible) {
    ensurePanel();
    switchCanView();
  }
}

/** Create the shared panel on first use, wiring row-activate → source jump. */
function ensurePanel(): void {
  if (canPanel || !canHostEl) return;
  canPanel = new CanPanel(canHostEl, {
    onRowActivate: (lineNumber) => revealSourceLine(lineNumber),
    onError: (e) => toastMsg(e),
  });
}

/** Show the active document's store+filter in the shared panel. */
function switchCanView(): void {
  if (!canPanel) return;
  const a = tabs.active();
  const docId = a?.info.docId ?? -1;
  // Reuse the doc's existing store (registered on first parse) instead of
  // minting a fresh empty one every switch, so re-attaching shows real frames.
  let store = docId >= 0 ? canStores.get(docId) : undefined;
  if (!store) {
    store = new FrameStore();
    if (docId >= 0) canStores.set(docId, store);
  }
  canPanel.attachStore(store);
  canPanel.setFilter((docId >= 0 && canFilters.get(docId)) || {});
  restoreFilterInputs(docId >= 0 ? canFilters.get(docId) : undefined);
}

/** Reflect a doc's filter back into the input row (blank when none). */
function restoreFilterInputs(f: CanFilter | undefined): void {
  const set = (k: string, v?: number | string) => {
    const el = canF[k];
    if (el) el.value = v === undefined ? "" : String(v);
  };
  set("idFrom", f?.idFrom);
  set("idTo", f?.idTo);
  set("msgName", f?.msgName);
  set("signalName", f?.signalName);
  set("signalValue", f?.signalValue);
  set("timeFromMs", f?.timeFromMs);
  set("timeToMs", f?.timeToMs);
  // Always reset the operator select (default `>` when the doc has no signal
  // filter) so a doc switch doesn't leave the previous doc's operator showing.
  const opEl = canF.signalOp as HTMLSelectElement | undefined;
  if (opEl) opEl.value = f?.signalOp ?? ">";
}

/** Frame row click → jump the matching source line (list 0-based, CM 1-based). */
function revealSourceLine(lineNumber: number): void {
  const a = tabs.active();
  if (!a) return;
  const list = lists.get(a.info.docId);
  if (list) list.revealLine(lineNumber - 1);
  else editors.get(a.info.docId)?.revealLine(lineNumber);
}

async function importDbc(): Promise<void> {
  const picked = await open({
    multiple: false,
    filters: [{ name: "DBC", extensions: ["dbc"] }],
  });
  if (typeof picked !== "string") return;
  try {
    const info: DbcInfo = await dbcImport(picked);
    lastDbcId = info.dbcId;
    const name = basenameOf(picked);
    canDbcLine!.textContent = `DBC：${name}（${info.messages.length} 报文）`;
    if (info.errors.length > 0) {
      toastMsg(`已宽容导入 ${name}，${info.errors.length} 处告警（坏信号行已跳过）`);
    } else {
      toastMsg(`已导入 ${name}`);
    }
  } catch (err) {
    reportError(err);
  }
}

function onCanRunClick(): void {
  if (canParsing) {
    // `can_parse` now returns its job id up front, so Stop is real: flip the
    // shared cancel flag the worker checks per frame. The tally still arrives on
    // `onDone` (a cancelled run sends partial stats), which resets the button.
    if (canReqId !== null) {
      canCancel(canReqId).catch((e) => reportError(e));
    }
    return;
  }
  void parseActive();
}

function updateCanExportUi(busy: boolean): void {
  canExportBusy = busy;
  for (const b of canExportBtns) b.disabled = busy || canParsing;
  if (canExportCancelBtn) canExportCancelBtn.style.display = busy ? "" : "none";
}

/** Export the active doc's CAN frame set (backend re-decode + filter, spec §7.5). */
async function onCanExport(format: "csv" | "jsonl"): Promise<void> {
  const a = tabs.active();
  if (!a) {
    toastMsg("请先打开一个文档");
    return;
  }
  if (canExportBusy || canParsing) {
    toastMsg("CAN 任务进行中，请稍候…");
    return;
  }
  const docId = a.info.docId;
  const ds = docStates.get(docId);
  const base = ds ? basenameOf(ds.path) : "can";
  const path = await save({
    defaultPath: `${base}.${format}`,
    filters: [{ name: format === "csv" ? "CSV" : "JSONL", extensions: [format] }],
  });
  if (typeof path !== "string") return;
  const sourceKind = canSrcSel!.value;
  const regex = sourceKind === "regex" ? canRegexInput!.value : null;
  const filter = canFilters.get(docId) ?? {};
  updateCanExportUi(true);
  canExportReqId = null;
  if (canExportStatus) canExportStatus.textContent = "导出中…";
  try {
    const args = {
      docId,
      dbcId: lastDbcId,
      sourceKind,
      regex,
      filter,
      targetPath: path,
      onProgress: (p: { written: number }) => {
        if (canExportStatus) canExportStatus.textContent = `导出中… ${p.written} 帧`;
      },
      onDone: (s: CanExportStats) => {
        updateCanExportUi(false);
        const msg = s.errored
          ? `导出中断（已写 ${s.written} 帧）`
          : s.cancelled
            ? `已取消，导出 ${s.written} 帧`
            : `已导出 ${s.written} 帧 → ${basenameOf(path)}`;
        if (canExportStatus) canExportStatus.textContent = msg;
        toastMsg(msg);
      },
    };
    canExportReqId = format === "csv" ? await exportCsv(args) : await exportJsonl(args);
  } catch (err) {
    updateCanExportUi(false);
    reportError(err);
  }
}

function cancelCanExport(): void {
  if (canExportReqId !== null) {
    exportCanCancel(canExportReqId).catch((e) => reportError(e));
  }
}

async function parseActive(): Promise<void> {
  // Re-entrancy guard lives here (not only in the button handler) so the command
  // palette's `can-parse` entry can't start a second run that double-fills the
  // table while one is already streaming. Shares the dock's single-flight with export.
  if (canParsing || canExportBusy) {
    toastMsg(canParsing ? "解析进行中，请稍候…" : "导出进行中，请稍候…");
    return;
  }
  const a = tabs.active();
  if (!a) {
    toastMsg("请先打开一个文档");
    return;
  }
  if (!canDockVisible) toggleCanDock();
  ensurePanel();
  const docId = a.info.docId;

  // Validate inputs BEFORE touching the store: a rejected launch (empty regex)
  // must leave the previous table intact rather than clear it and stream nothing.
  const sourceKind = canSrcSel!.value;
  const regex = sourceKind === "regex" ? canRegexInput!.value : null;
  if (sourceKind === "regex" && regex === "") {
    toastMsg("正则源需填写含命名组 id/data 的正则");
    return;
  }

  const store = canStores.get(docId) ?? new FrameStore();
  canStores.set(docId, store);
  store.clear(); // a re-parse replaces the table wholesale
  canPanel!.attachStore(store);
  canPanel!.setFilter(canFilters.get(docId) ?? {});
  setCanParsing(true);
  canReqId = null;
  try {
    // Resolves as soon as the shell has validated + spawned the worker (frames
    // keep streaming on the channels afterward), giving us the id for Stop.
    canReqId = await canParse({
      docId,
      dbcId: lastDbcId,
      sourceKind,
      regex,
      onBatch: (frames: FrameJson[]) => {
        store.push(frames);
        canPanel?.render();
      },
      onDone: (s) => {
        toastMsg(`解码 ${s.decoded} 帧，跳过 ${s.skipped} 行`);
        canPanel?.render();
        // Completion (including a cancelled run) is the only place parsing ends;
        // the promise itself resolves early, so `finally` must not reset here.
        setCanParsing(false);
      },
    });
    // Only flip to Stop if the run is still going (a tiny file can finish before
    // this line, where onDone has already reset the button).
    if (canParsing && canRunBtn) canRunBtn.textContent = "停止";
  } catch (err) {
    reportError(err);
    setCanParsing(false); // a rejected start never streams, so reset here
  }
}

function setCanParsing(b: boolean): void {
  canParsing = b;
  if (!b) canReqId = null;
  if (canRunBtn) canRunBtn.textContent = b ? "解析中…" : "解析报文";
}

/** Editor-selection → one frame, surfaced as a toast ("识别为报文"). */
async function decodeSelection(): Promise<void> {
  const a = tabs.active();
  if (!a) {
    toastMsg("请先打开一个文档");
    return;
  }
  const handle = editors.get(a.info.docId);
  if (!handle) {
    toastMsg("只读视图不支持选中文本识别");
    return;
  }
  const sel = handle.view.state.selection.main;
  const text = handle.view.state.sliceDoc(sel.from, sel.to).trim();
  if (text === "") {
    toastMsg("请先在编辑器中选中一行报文");
    return;
  }
  try {
    // Pass the active source kind so an ASC selection decodes as ASC (mirrors
    // the dock's `源` select); a regex selection is rejected by the shell with a
    // clear `source_format` toast rather than silently mis-parsed as candump.
    const f = await canDecodeSnippet({ text, dbcId: lastDbcId, sourceKind: canSrcSel?.value ?? null });
    const sig = f.signals.length ? `，${f.signals.length} 信号` : "";
    toastMsg(`报文 ${f.canIdHex}${f.msgName ? " " + f.msgName : ""}：${f.dlc} 字节${sig}`);
  } catch (err) {
    reportError(err);
  }
}

async function openFile(): Promise<void> {
  const picked = await open({
    multiple: false,
    filters: [{ name: "日志/文本", extensions: ["*"] }],
  });
  if (typeof picked === "string") await openPath(picked);
}

async function openPath(path: string): Promise<boolean> {
  if (busy) return false;
  setBusy(true);
  try {
    const info = await docOpen(path, onIndexProgress);
    // The shell owns the ≤200MB split (spec §4.3/A1) via `DocInfo.mode`.
    if (info.mode === "readonly") mountList(info, path);
    else await mountDoc(info, path);
    return true;
  } catch (err) {
    reportError(err);
    return false;
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

/** Write the dirty doc's shadow after the 2s idle debounce (best-effort). */
function fireRecovery(docId: number): void {
  const ds = docStates.get(docId);
  const h = editors.get(docId);
  if (!ds || !h || !h.isDirty()) return;
  const path = ds.path;
  let write: Promise<void>;
  write = recoverySave({
    path,
    encoding: ds.encoding,
    eol: ds.eol,
    text: h.getText(),
  })
    .catch(() => {
      // A shadow write failure must never interrupt editing; the next idle retries.
    })
    .finally(() => {
      if (recoveryWrites.get(path) === write) recoveryWrites.delete(path);
    });
  recoveryWrites.set(path, write);
}

/** Arm the shadow timer for a dirty edit-mode doc (no-op when disabled). */
function scheduleRecovery(docId: number): void {
  if (!recoveryOn) return;
  const h = editors.get(docId);
  if (!h || !h.isDirty()) return;
  recoveryDebounce.schedule(docId);
}

/** Drop a pending shadow write and delete the on-disk shadow for `path`. */
function cancelRecovery(docId: number, path: string): void {
  recoveryDebounce.cancel(docId);
  const inflight = recoveryWrites.get(path);
  if (inflight) {
    // A write may still be landing: delete only after it settles, else a stale
    // shadow could survive a successful save/close.
    void inflight.finally(() => recoveryDiscard(path).catch(() => {}));
  } else {
    recoveryDiscard(path).catch(() => {});
  }
  // Keep the banner in sync when the shadow is cleared from outside it.
  if (recoveryEntries.some((e) => e.path === path)) {
    recoveryEntries = recoveryEntries.filter((e) => e.path !== path);
    renderRecoveryBanner();
  }
}

// ---- crash recovery UI (spec §4.4/§10, M3 B1) -------------------------------

function removeRecoveryBanner(): void {
  recoveryBannerEl?.remove();
  recoveryBannerEl = null;
}

function renderRecoveryBanner(): void {
  removeRecoveryBanner();
  if (recoveryEntries.length === 0) return;
  const box = document.createElement("div");
  box.className = "recovery-banner";
  box.setAttribute("role", "status");
  const head = document.createElement("div");
  head.className = "recovery-head";
  head.textContent = `发现 ${recoveryEntries.length} 个未保存恢复项（崩溃恢复）`;
  const list = document.createElement("div");
  list.className = "recovery-list";
  for (const e of recoveryEntries) {
    const row = document.createElement("div");
    row.className = "recovery-row";
    const label = document.createElement("span");
    label.className = "recovery-label";
    label.textContent = `${basenameOf(e.path)} · ${formatSavedAt(e.savedAtMs)} · ${humanSize(e.sizeBytes)}`;
    label.title = e.path;
    const ok = mkBtn("恢复", "把影子内容恢复到编辑器（恢复后仍未保存）", () => void restoreRecovery(e));
    const no = mkBtn("丢弃", "删除该影子，不再提示", () => void discardRecovery(e.path));
    row.append(label, ok, no);
    list.append(row);
  }
  const foot = document.createElement("div");
  foot.className = "recovery-foot";
  foot.append(
    mkBtn("全部丢弃", "删除全部恢复项", () => void discardAllRecovery()),
    mkBtn("关闭", "暂时收起（影子保留，下次启动再问）", () => removeRecoveryBanner()),
  );
  box.append(head, list, foot);
  document.body.append(box);
  recoveryBannerEl = box;
}

async function checkRecovery(): Promise<void> {
  if (!recoveryOn) {
    removeRecoveryBanner();
    return;
  }
  try {
    recoveryEntries = await recoveryList();
  } catch {
    return; // a config/dir problem must never block startup
  }
  renderRecoveryBanner();
}

async function discardRecovery(path: string): Promise<void> {
  try {
    await recoveryDiscard(path);
  } catch {
    // idempotent backend: failure only means the file is already gone
  }
  recoveryEntries = recoveryEntries.filter((e) => e.path !== path);
  renderRecoveryBanner();
}

async function discardAllRecovery(): Promise<void> {
  for (const e of [...recoveryEntries]) {
    await recoveryDiscard(e.path).catch(() => {});
  }
  recoveryEntries = [];
  renderRecoveryBanner();
}

/** Open the original file, then replace its text with the shadow (kept dirty). */
async function restoreRecovery(e: RecoveryEntry): Promise<void> {
  if (busy) {
    toastMsg("请稍候…");
    return;
  }
  const existing = tabs.list().find((t) => t.path === e.path);
  if (existing) {
    // Reuse the open tab: a second doc id for the same path would fork the
    // shadow key and fight over the same file on save.
    tabs.activate(existing.docId);
  } else if (!(await openPath(e.path))) {
    toastMsg(`原文件已缺失，无法恢复：${basenameOf(e.path)}`);
    return; // keep the entry: only an explicit discard removes it
  }
  const a = tabs.active();
  const docId = a?.info.docId;
  const h = docId === undefined ? undefined : editors.get(docId);
  const ds = docId === undefined ? undefined : docStates.get(docId);
  if (docId === undefined || !h || !ds || ds.path !== e.path) {
    toastMsg("恢复失败：未找到刚打开的文档");
    return;
  }
  try {
    const c = await recoveryLoad(e.path);
    h.replaceText(c.text); // triggers onEdit -> a fresh shadow after the next idle
    ds.encoding = c.encoding || ds.encoding;
    ds.eol = c.eol || ds.eol;
    status.setEncoding(ds.encoding);
    status.setEol(ds.eol);
    tabs.refreshDirty(docId);
    updateInspector(docId);
    refreshSidebar();
    // Drop the consumed shadow; the dirty editor re-writes it after 2s idle.
    await recoveryDiscard(e.path).catch(() => {});
    recoveryEntries = recoveryEntries.filter((x) => x.path !== e.path);
    renderRecoveryBanner();
    toastMsg(`已恢复 ${basenameOf(e.path)}（仍未保存）`);
  } catch (err) {
    reportError(err);
  }
}

async function onRecoveryToggle(want: boolean): Promise<void> {
  try {
    await recoverySetEnabled(want);
    recoveryOn = want;
    if (want) {
      void checkRecovery();
    } else {
      recoveryDebounce.cancelAll();
      recoveryEntries = [];
      removeRecoveryBanner();
    }
  } catch (err) {
    if (recoveryChk) recoveryChk.checked = !want; // roll back the checkbox
    reportError(err);
  }
}

async function initRecovery(): Promise<void> {
  try {
    recoveryOn = await recoveryEnabled();
  } catch {
    recoveryOn = true; // default on when the config cannot be read
  }
  if (recoveryChk) recoveryChk.checked = recoveryOn;
  if (recoveryOn) void checkRecovery();
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
    // onDirty fires once per dirty cycle; every edit must re-arm the 2s timer.
    onEdit: () => scheduleRecovery(info.docId),
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
    loadLines: (indices) => fetchLines(info.docId, indices),
    onInvalidCount: (n) => {
      // §5.4: every fetched window reports its unmappable count; keep the
      // status-bar ⚠ in sync (only repaint if this doc is the shown one).
      const d = docStates.get(info.docId);
      if (d) d.invalid = n;
      if (info.docId === tabs.active()?.info.docId) {
        status.setInvalidCount(n);
        updateInspector(info.docId);
      }
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
  const prevPath = ds.path; // shadow key before a possible Save-As rebind

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
    afterSave(a.info.docId, handle, ds, report.replaceCount, report.conflict, false, target, prevPath);
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
          afterSave(a.info.docId, handle, ds, report.replaceCount, report.conflict, true, target, prevPath);
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
  prevPath: string,
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
  // Saved: the shadow is stale — clear both the pre-Save-As and the new path.
  cancelRecovery(docId, prevPath);
  cancelRecovery(docId, ds.path);
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
      // cu16 ranges + the kept-row set were computed against the OLD decoding, so
      // a filter/highlight result is meaningless now — drop it (review #4).
      list.setView(null, false, false);
      queryHits.delete(docId);
      queryCursor.delete(docId);
      toastMsg("编码已变更，请重新应用查询");
    }
    ds.encoding = updated.encoding;
    ds.eol = updated.eol || ds.eol;
    status.setEncoding(updated.encoding);
    updateInspector(docId); // inspector shows the new encoding / ⚠ detail
    if (handle) {
      tabs.refreshDirty(docId);
      // The dirty text was discarded by the re-decode; any shadow is stale.
      cancelRecovery(docId, ds.path);
    }
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
  const closing = docStates.get(docId);
  if (closing) cancelRecovery(docId, closing.path);
  tabs.remove(docId);
  docStates.delete(docId);
  editors.delete(docId);
  lists.delete(docId);
  indexed.delete(docId);
  canStores.delete(docId);
  canFilters.delete(docId); // a closed doc drops its table; the panel is rebuilt on demand
  queryHits.delete(docId); // drop the doc's filter hits/cursor (spec §4.1) — else a
  queryCursor.delete(docId); // late `filter_run` onDone repopulates a closed, never-reused id
  refreshPanels(); // after the maps drop, so a closed doc leaves the sidebar
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

// ---- Query panel: multi-condition line filter + highlight (M3 spec §4.1).
// This replaces the M1 single-box read-only search (Ctrl+Shift+F). The panel
// drives `filter_run` (AND of include/exclude × literal/regex, case toggle);
// "过滤到命中行" toggles subset rendering, "高亮命中" overlays cu16 ranges, and
// F3/Shift+F3 cycle hits. `search_run` stays registered on the shell but is no
// longer the front-end path (plan Ruling: a single filter_run include-condition
// subsumes it).

const queryHits = new Map<number, LineHit[]>();
const queryCursor = new Map<number, number>();
// One scan at a time across all docs — an intentional v1 global mutex (a second
// concurrent scan would fight over the single Stop/`queryReqId` affordance).
let queryBusy = false;
let queryReqId: number | null = null;
let queryKind: "scan" | "export" = "scan"; // which command the shared Stop button cancels

/** Promise wrapper over the channel-based `doc_lines_at` (the subset view's loader). */
function fetchLines(
  docId: number,
  indices: number[],
): Promise<{ lines: IndexedText[]; invalidCount: number }> {
  return new Promise((resolve, reject) => {
    if (indices.length === 0) {
      resolve({ lines: [], invalidCount: 0 });
      return;
    }
    docLinesAt({ docId, indices, onData: (p) => resolve(p) }).catch(reject);
  });
}

function addCondRow(conds: HTMLElement, onEnter: () => void): void {
  const row = document.createElement("div");
  row.className = "query-cond";
  const mk = (opts: [string, string][], role: string): HTMLSelectElement => {
    const s = document.createElement("select");
    s.dataset.role = role;
    for (const [v, t] of opts) {
      const o = document.createElement("option");
      o.value = v;
      o.textContent = t;
      s.append(o);
    }
    return s;
  };
  const mode = mk([["include", "包含"], ["exclude", "排除"]], "mode");
  const kind = mk([["literal", "字面"], ["regex", "正则"]], "kind");
  const inp = document.createElement("input");
  inp.type = "text";
  inp.placeholder = "文本 / 正则";
  inp.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing) onEnter();
  });
  const del = document.createElement("button");
  del.type = "button";
  del.textContent = "×";
  del.addEventListener("click", () => row.remove());
  row.append(mode, kind, inp, del);
  conds.append(row);
}

function collectConditions(conds: HTMLElement): FilterCondition[] {
  const out: FilterCondition[] = [];
  for (const child of Array.from(conds.children) as HTMLElement[]) {
    const mode = child.querySelector('[data-role="mode"]') as HTMLSelectElement | null;
    const kind = child.querySelector('[data-role="kind"]') as HTMLSelectElement | null;
    const inp = child.querySelector("input") as HTMLInputElement | null;
    const text = (inp?.value ?? "").trim();
    if (text === "") continue;
    out.push({
      mode: mode?.value === "exclude" ? "exclude" : "include",
      kind: kind?.value === "regex" ? "regex" : "literal",
      text,
    });
  }
  return out;
}

async function applyQuery(
  statusEl: HTMLElement,
  stopBtn: HTMLButtonElement,
  conditions: FilterCondition[],
  caseSensitive: boolean,
  hideNon: boolean,
  highlight: boolean,
  sel: TimelineSelection,
): Promise<void> {
  const a = tabs.active();
  if (!a) return;
  const docId = a.info.docId;
  const list = lists.get(docId);
  if (sel.error) {
    toastMsg(sel.error);
    return;
  }
  const hasAxis =
    conditions.length > 0 || sel.time !== undefined || sel.lineFrom !== undefined || sel.lineTo !== undefined;
  if (!hasAxis) {
    toastMsg("请至少添加一条条件或时间/行号约束");
    return;
  }
  if (!list) {
    toastMsg("仅只读大文件视图支持行过滤");
    return;
  }
  if (queryBusy) {
    toastMsg("查询进行中，请稍候…");
    return;
  }
  queryBusy = true;
  queryKind = "scan";
  queryReqId = null;
  stopBtn.disabled = false;
  statusEl.textContent = "扫描中…";
  const hits: LineHit[] = [];
  const finish = (text: string) => {
    queryBusy = false;
    queryReqId = null;
    stopBtn.disabled = true;
    statusEl.textContent = text;
  };
  try {
    // Resolves as soon as the shell has validated + spawned the worker; batches
    // keep streaming on the channels afterward (gives us the id for Stop).
    queryReqId = await filterRun({
      docId,
      conditions,
      caseSensitive,
      time: sel.time ?? null,
      lineFrom: sel.lineFrom ?? null,
      lineTo: sel.lineTo ?? null,
      onBatch: (b) => {
        hits.push(...b);
        statusEl.textContent = `已命中 ${hits.length} 行…`;
      },
      onDone: (s) => {
        if (!docStates.has(docId)) {
          finish("查询已取消"); // doc closed mid-scan: don't repopulate leaked state
          return;
        }
        queryHits.set(docId, hits);
        queryCursor.set(docId, hits.length ? 0 : -1);
        list.setView(hits, hideNon, highlight);
        finish(`命中 ${s.kept}/${s.scanned} 行` + (s.truncated ? "（高亮已截断）" : ""));
      },
    });
    if (!queryBusy) queryReqId = null; // onDone already ran (empty/tiny scan beat the resolve)
  } catch (err) {
    reportError(err);
    finish("查询失败");
  }
}

/** Export the currently-selected lines to `targetPath` (raw bytes, spec §7.5). */
async function exportQuery(
  statusEl: HTMLElement,
  stopBtn: HTMLButtonElement,
  conditions: FilterCondition[],
  caseSensitive: boolean,
  sel: TimelineSelection,
  targetPath: string,
): Promise<void> {
  const a = tabs.active();
  if (!a) return;
  const docId = a.info.docId;
  if (queryBusy) {
    toastMsg("查询进行中，请稍候…");
    return;
  }
  queryBusy = true;
  queryKind = "export";
  queryReqId = null;
  stopBtn.disabled = false;
  statusEl.textContent = "导出中…";
  const finish = (text: string) => {
    queryBusy = false;
    queryReqId = null;
    stopBtn.disabled = true;
    statusEl.textContent = text;
  };
  try {
    queryReqId = await exportLines({
      docId,
      conditions,
      caseSensitive,
      time: sel.time ?? null,
      lineFrom: sel.lineFrom ?? null,
      lineTo: sel.lineTo ?? null,
      targetPath,
      onProgress: (p) => {
        statusEl.textContent = `导出中… ${p.written} 行`;
      },
      onDone: (s) => {
        finish(
          s.errored
            ? `导出中断（已写 ${s.written} 行）`
            : s.cancelled
              ? `已取消，导出 ${s.written} 行`
              : `导出 ${s.written} 行`,
        );
      },
    });
    if (!queryBusy) queryReqId = null;
  } catch (err) {
    reportError(err);
    finish("导出失败");
  }
}

function cycleHit(dir: number): void {
  const a = tabs.active();
  if (!a) return;
  const hits = queryHits.get(a.info.docId);
  if (!hits || hits.length === 0) {
    toastMsg("尚无命中，请先应用查询");
    return;
  }
  const list = lists.get(a.info.docId);
  if (!list) {
    toastMsg("仅只读大文件视图支持跳转");
    return;
  }
  let c = (queryCursor.get(a.info.docId) ?? -1) + dir;
  if (c < 0) c = hits.length - 1;
  if (c >= hits.length) c = 0;
  queryCursor.set(a.info.docId, c);
  list.scrollToHit(c);
}

function openQueryPanel(): void {
  const a = tabs.active();
  if (!a) {
    toastMsg("请先打开一个文档");
    return;
  }
  const panel = document.getElementById("search-panel");
  if (!panel) return;
  if (queryBusy && panel.childElementCount > 0) {
    // A scan is running: re-show the live panel so its Stop/status stay bound
    // rather than rebuilding and orphaning the in-flight run (review #5).
    panel.style.display = "";
    (panel.querySelector(".query-cond input") as HTMLInputElement | null)?.focus();
    return;
  }
  panel.textContent = "";
  panel.style.display = "";

  const head = document.createElement("div");
  head.className = "search-head";
  const conds = document.createElement("div");
  conds.className = "query-conds";
  const statusEl = document.createElement("span");
  statusEl.className = "search-status";

  const chk = (label: string, on: boolean, host: HTMLElement): HTMLInputElement => {
    const wrap = document.createElement("label");
    wrap.className = "query-chk";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = on;
    wrap.append(box, document.createTextNode(label));
    host.append(wrap);
    return box;
  };
  const hideBox = chk("过滤到命中行", true, head);
  const hlBox = chk("高亮命中", true, head);
  const caseBox = chk("区分大小写", false, head);

  let stopBtn: HTMLButtonElement; // assigned before any handler runs
  const btn = (label: string, onClick: () => void): HTMLButtonElement => {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = label;
    b.addEventListener("click", onClick);
    head.append(b);
    return b;
  };

  // Timeline axis row (spec §2): extract regex + built-in format + [from,to] +
  // a keep-no-ts toggle. Blank extract => [from,to] is a 1-based line range.
  const timeRow = document.createElement("div");
  timeRow.className = "query-time";
  timeRow.append(document.createTextNode("时间戳 "));
  const tsIn = document.createElement("input");
  tsIn.type = "text";
  tsIn.placeholder = "抽取正则 (?<ts>…) 空则按行号";
  const fmtSel = document.createElement("select");
  for (const [v, t] of [["iso", "ISO"], ["epoch_s", "epoch秒"], ["epoch_ms", "epoch毫秒"]]) {
    const o = document.createElement("option");
    o.value = v;
    o.textContent = t;
    fmtSel.append(o);
  }
  const tFrom = document.createElement("input");
  tFrom.type = "text";
  tFrom.placeholder = "起";
  const tTo = document.createElement("input");
  tTo.type = "text";
  tTo.placeholder = "止";
  const keepChk = chk("保留无时间戳行", false, timeRow);
  timeRow.append(tsIn, fmtSel, tFrom, tTo);

  const currentSelection = (): TimelineSelection =>
    pickTimelineInput(tsIn.value, fmtSel.value as TimeFormat, tFrom.value, tTo.value, keepChk.checked);

  const apply = () =>
    void applyQuery(
      statusEl,
      stopBtn,
      collectConditions(conds),
      caseBox.checked,
      hideBox.checked,
      hlBox.checked,
      currentSelection(),
    );
  btn("＋条件", () => addCondRow(conds, apply));
  btn("应用", apply);
  btn("上一个", () => cycleHit(-1));
  btn("下一个", () => cycleHit(1));
  const doExport = async () => {
    const sel = currentSelection();
    if (sel.error) {
      toastMsg(sel.error);
      return;
    }
    // Export reads the on-disk DocFile; editable docs' unsaved buffer isn't on
    // disk, so the raw-byte export would silently emit pre-edit bytes. Guard to
    // the read-only view (mirrors applyQuery's list check).
    const cur = tabs.active();
    if (!cur || !lists.has(cur.info.docId)) {
      toastMsg("仅只读大文件视图支持导出");
      return;
    }
    const conditions = collectConditions(conds);
    const hasAxis =
      conditions.length > 0 || sel.time !== undefined || sel.lineFrom !== undefined || sel.lineTo !== undefined;
    if (!hasAxis) {
      const ok = await ask("当前无筛选，将导出全部行，确定？", {
        title: "导出",
        kind: "warning",
        okLabel: "导出",
        cancelLabel: "取消",
      });
      if (!ok) return;
    }
    const ds = docStates.get(cur.info.docId);
    const base = ds ? basenameOf(ds.path) : "log";
    const path = await save({ defaultPath: `${base}.filtered.log` });
    if (typeof path !== "string") return;
    void exportQuery(statusEl, stopBtn, conditions, caseBox.checked, sel, path);
  };
  btn("导出…", () => void doExport());
  stopBtn = btn("停止", () => {
    if (queryReqId !== null) {
      const p = queryKind === "export" ? exportCancel(queryReqId) : filterCancel(queryReqId);
      void p.catch(reportError);
    }
  });
  stopBtn.disabled = true;
  btn("关闭", hideSearchPanel);

  panel.append(head, conds, timeRow, statusEl);
  addCondRow(conds, apply);
  (conds.querySelector("input") as HTMLInputElement | null)?.focus();
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
  document.getElementById("app")?.append(panel);
  input.focus();
  input.select();
}

/** The command set offered by the palette, rebuilt per open so it reflects
 *  live state (e.g. hints). Titles are what `fuzzyScore` matches. */
function buildCommands(): Command[] {
  return [
    { id: "open", title: "打开文件…", hint: "Ctrl+O", run: () => void openFile() },
    { id: "save", title: "保存", hint: "Ctrl+S", run: () => void saveActive(false) },
    { id: "save-as", title: "另存为…", hint: "Ctrl+Shift+S", run: () => void saveActive(true) },
    { id: "query", title: "行过滤 / 高亮命中…", hint: "Ctrl+Shift+F", run: () => openQueryPanel() },
    { id: "jump", title: "跳转到行…", hint: "Ctrl+G", run: () => openJumpPrompt() },
    {
      id: "close",
      title: "关闭当前文档",
      run: () => {
        const a = tabs.active();
        if (a) void handleClose(a.info.docId);
      },
    },
    { id: "sidebar", title: "显示 / 隐藏侧栏", run: () => toggleSidebar() },
    { id: "inspector", title: "显示 / 隐藏文档信息栏", run: () => toggleInspector() },
    { id: "can-panel", title: "报文面板：显示 / 隐藏", run: () => toggleCanDock() },
    { id: "can-import", title: "导入 DBC…", run: () => void importDbc() },
    { id: "can-parse", title: "解析当前文档为报文", run: () => void parseActive() },
    { id: "can-snippet", title: "识别选中为报文", run: () => void decodeSelection() },
    { id: "theme-light", title: "主题：浅色", run: () => theme.set("light") },
    { id: "theme-dark", title: "主题：暗黑", run: () => theme.set("dark") },
    { id: "theme-vampire", title: "主题：吸血鬼 (Dracula)", run: () => theme.set("vampire") },
    { id: "theme-auto", title: "主题：跟随系统", run: () => theme.set("auto") },
  ];
}

/** Ctrl+Shift+P palette: a fuzzy-filtered, keyboard-navigable command list. */
function openPalette(): void {
  const already = document.getElementById("palette");
  if (already) {
    (already.querySelector("input") as HTMLInputElement | null)?.focus();
    return;
  }
  const overlay = document.createElement("div");
  overlay.id = "palette";
  overlay.className = "palette-overlay";
  const box = document.createElement("div");
  box.className = "palette-box";
  const input = document.createElement("input");
  input.type = "text";
  input.className = "palette-input";
  input.placeholder = "输入命令…";
  const list = document.createElement("div");
  list.className = "palette-list";
  box.append(input, list);
  overlay.append(box);
  document.getElementById("app")?.append(overlay);

  const cmds = buildCommands();
  let view: Command[] = filterCommands("", cmds);
  let sel = 0;

  const paintSel = () => {
    Array.from(list.children).forEach((el, i) =>
      el.classList.toggle("sel", i === sel),
    );
  };
  const close = () => overlay.remove();
  const run = (i: number) => {
    const c = view[i];
    close();
    c?.run();
  };
  const render = () => {
    list.textContent = "";
    view.forEach((c, i) => {
      const row = document.createElement("div");
      row.className = "palette-row" + (i === sel ? " sel" : "");
      const t = document.createElement("span");
      t.className = "palette-title";
      t.textContent = c.title;
      row.append(t);
      if (c.hint) {
        const h = document.createElement("span");
        h.className = "palette-hint";
        h.textContent = c.hint;
        row.append(h);
      }
      row.addEventListener("mouseenter", () => {
        sel = i;
        paintSel();
      });
      row.addEventListener("click", () => run(i));
      list.append(row);
    });
  };
  const update = (q: string) => {
    view = filterCommands(q, cmds);
    sel = 0;
    render();
  };

  input.addEventListener("input", () => update(input.value));
  input.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      close();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (view.length) sel = (sel + 1) % view.length;
      paintSel();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (view.length) sel = (sel - 1 + view.length) % view.length;
      paintSel();
    } else if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      if (view.length) run(sel);
    }
  });
  overlay.addEventListener("mousedown", (e) => {
    if (e.target === overlay) close();
  });

  render();
  input.focus();
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
    if (e.key === "F3") {
      // Only the read-only list view cycles query hits; in an editor let
      // CodeMirror's own searchKeymap handle F3 (avoids a double response).
      const a = tabs.active();
      if (a && lists.has(a.info.docId)) {
        e.preventDefault();
        cycleHit(e.shiftKey ? -1 : 1);
      }
      return;
    }
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
      openQueryPanel();
    } else if (key === "p" && e.shiftKey) {
      e.preventDefault();
      openPalette(); // Ctrl+Shift+P command palette (UI reference: NoteLume)
    }
  });
}

layout();
detectChrome();
registerShortcuts();
// First-interaction probe (spec P1/P2): the app_ping round-trip is the wired
// timing endpoint for the manual GUI cold-start measurement. A headless CI
// cannot observe real first-paint, so §2 records P1/P2 as human smoke items.
void appPing().catch(() => {});
void initRecovery();
