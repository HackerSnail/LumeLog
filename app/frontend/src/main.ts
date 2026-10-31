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
  canParseBlf,
  canCancel,
  canDecodeSnippet,
  filterRun,
  filterCancel,
  docLinesAt,
  exportLines,
  exportCancel,
  exportCsv,
  exportJsonl,
  exportBlfCsv,
  exportBlfJsonl,
  exportCanCancel,
  recoverySave,
  recoveryDiscard,
  recoveryList,
  recoveryLoad,
  recoveryEnabled,
  recoverySetEnabled,
  watchSubscribe,
  docReload,
  watchStatus,
  keymapGet,
  keymapSet,
  keymapReset,
  docTailAppend,
  scriptList,
  scriptRead,
  scriptWrite,
  scriptDelete,
  scriptRun,
  scriptCancel,
  mf4Open,
  mf4Close,
  mf4Channels,
  mf4Summary,
  mf4Records,
  mf4ExportCsv,
  mf4ExportCancel,
  mf4Series,
  type ExternalEvent,
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
  syntaxPluginsList,
  parserPluginsList,
  parserRun,
  parserCancel,
  viewerOpen,
  viewerRows,
  viewerClose,
  viewerExportCsv,
  type ParserPluginInfo,
  type ViewerInfo,
  recentList,
  recentClear,
  viewerPdfBytes,
  gitInit,
  gitProbe,
  gitCommit,
} from "./ipc";
import { ParserPanel, RowStore } from "./parserpanel";
import { PdfView, clampPage, stepZoom } from "./pdfview";
import { pickTimelineInput, type TimelineSelection } from "./query";
import { Debouncer, formatSavedAt, humanSize } from "./recovery";
import { eventToAccel, DEFAULT_BINDINGS, mergeBindings, type ActionId } from "./keymap";
import { openKeymapModal } from "./keymap-ui";
import { openScriptModal } from "./script-ui";
import { Mf4View, type Mf4Query } from "./mf4-ui";
import { openEditable, type EditorHandle } from "./editor";
import {
  MACRO_STORE_MAX,
  parseMacros,
  parseRepeatCount,
  serializeMacros,
  type NamedMacro,
} from "./editortools";
import { ListView, isNearBottom } from "./listview";
import { CanPanel, FrameStore } from "./canpanel";
import { type CanFilter } from "./canfilter";
import { TabStrip } from "./tabs";
import { Statusbar } from "./statusbar";
import { extOf } from "./encoding-menu";
import { autoModeId, registerPluginLanguages, syntaxModes, type ModeId } from "./language";
import { initTheme } from "./theme";
import {
  makeResizable,
  loadFlag,
  storeFlag,
  isIMEEvent,
  MIN_SIDEBAR,
  MAX_SIDEBAR,
  MIN_INSPECTOR,
  MAX_INSPECTOR,
} from "./shell";
import { filterCommands, type Command } from "./palette";
import { parseBookmarkStore, serializeBookmarkStore, type BookmarkStore } from "./bookmarks-store";
import { macroForAccel, parseMacroBindings, serializeMacroBindings, type MacroBindings } from "./macrokeys";
import { normalizeSession, parseSession, serializeSession } from "./session";
import {
  addTag,
  parseTagStore,
  removeTag,
  serializeTagStore,
  TAGS_PER_DOC_MAX,
  type TagStore,
} from "./doctags";
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
  /** Open-time file size / mtime (Plan 33 inspector rows); stale after tail growth. */
  size: number;
  mtime: number;
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
// External-watch reload single-flight (per doc) + one non-blocking bar.
const watchReloading = new Set<number>();
const watchPending = new Set<number>();
let extBarEl: HTMLElement | null = null;
let extBarDocId: number | null = null;
// Tail follow (Roadmap): per-doc on/paused state + one owner-bound "back to bottom" chip.
const followStates = new Map<number, { on: boolean; paused: boolean }>();
let followResumeEl: HTMLButtonElement | null = null;
let followResumeOwner: number | null = null;
const tailToast = new Map<number, { pending: number; at: number }>();
const tailSeenSize = new Map<number, number>();
// Keymap: raw config overrides + the merged view used for dispatch/palette hints.
let keymapOverrides: Record<string, string> = {};
let keymapState = mergeBindings({});
let keymapModalOpen = false;
let scriptModalOpen = false;

// ---- CanPanel / CAN decode state (spec §7.4) --------------------------------
// One panel instance is re-attached to whichever document is active; each
// document keeps its own accumulated frames + last filter so switching tabs
// restores that doc's table instead of the previous one's.
const canStores = new Map<number, FrameStore>();
const canFilters = new Map<number, CanFilter>();
// BLF view (Roadmap-2): no source document; its store/filter use a pseudo key.
const BLF_KEY = -2;
let blfView: { path: string; store: FrameStore } | null = null;
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
    saveSession();
    if (id >= 0 && blfView) {
      blfView = null; // switching to a real document leaves the BLF view
      if (canDockVisible) switchCanView();
    }
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
  onFollow: (on) => setFollow(on),
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
    mkBtn("查询", "行过滤 / 高亮命中", () => openQueryPanel()),
    mkBtn("报文面板", "显示 / 隐藏 CAN 报文面板", () => toggleCanDock()),
    mkBtn("测量", "打开 MF4 测量文件（表格浏览与导出）", () => void openMf4File()),
    mkBtn("查看", "查看 xlsx / docx 文档（表格与文本）", () => void openViewerFile()),
    mkBtn("脚本", "打开脚本管理器（QuickJS 逐行变换）", () => openScriptManager()),
    mkBtn("解析", "用解析插件把当前文档解析为表格（plugins 目录）", () => toggleParserDock()),
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
  // A shortcut-free entry: the palette itself may be unbound/cleared.
  const keymapBtn = mkBtn("快捷键设置…", "自定义快捷键", () => openKeymapSettings());
  keymapBtn.className = "side-action";
  aside.append(asideTitle, sidebarListEl, recoveryOption, keymapBtn);

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
  center.append(tabsEl, panel, buildCanDock(), buildMf4Dock(), buildParserDock(), buildViewerDock());

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

  root.append(bar, workspace, statusEl, toast, dropOverlay());
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

/** Fill the inspector with the active document's metadata (spec §5.4 ⚠ detail)
 *  plus per-path tags (Plan 33, NoteLume-style sections). */
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
  body.append(inspSection("文档属性"));
  const rows: [string, string, boolean?][] = [
    ["模式", ds.mode === "edit" ? "编辑（可修改）" : "只读（大文件）"],
    ["大小", humanSize(ds.size)],
    ["修改时间", formatSavedAt(ds.mtime)],
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

  body.append(inspSection("标签"));
  body.append(buildTagBlock(ds.path));

  body.append(inspSection("Git"));
  const gitBox = document.createElement("div");
  gitBox.id = "insp-git";
  gitBox.dataset.path = ds.path;
  gitBox.className = "insp-empty";
  gitBox.textContent = "检测中…";
  body.append(gitBox);
  void fillGitSection(ds.path);
}

/** Async half of the inspector Git section (Plan 34): probe then fill.
 *  `updateInspector` is sync, so the subprocess result lands afterwards;
 *  the dataset guard drops results for a doc that is no longer shown. */
async function fillGitSection(path: string): Promise<void> {
  const token = ++gitProbeToken;
  let info;
  try {
    info = await gitProbe(path);
  } catch (err) {
    info = null;
    reportError(err);
  }
  const box = document.getElementById("insp-git");
  if (!box || box.dataset.path !== path || token !== gitProbeToken) return;
  box.textContent = "";
  if (!info) return;
  if (info.unavailable) {
    box.textContent = info.unavailable;
    return;
  }
  if (!info.repo) {
    const msg = document.createElement("div");
    msg.className = "insp-empty";
    msg.textContent = "未检测到 Git 仓库";
    const btn = document.createElement("button");
    btn.className = "insp-git-init";
    btn.textContent = "初始化仓库";
    btn.addEventListener("click", () => void initRepoFor(path));
    box.append(msg, btn);
    return;
  }
  const rows: [string, string][] = [
    ["仓库根", info.root ?? "—"],
    ["分支", info.branch ?? "—"],
    ["文件状态", info.file_status ?? "未改动"],
  ];
  if (info.last_commit) {
    rows.push([
      "最近提交",
      `${info.last_commit.hash} ${info.last_commit.subject}（${formatSavedAt(info.last_commit.time * 1000)}）`,
    ]);
  } else {
    rows.push(["最近提交", "（尚无提交）"]);
  }
  for (const [key, value] of rows) {
    const row = document.createElement("div");
    row.className = "insp-row";
    const k = document.createElement("span");
    k.className = "insp-key";
    k.textContent = key;
    const v = document.createElement("span");
    v.className = "insp-val";
    v.textContent = value;
    v.title = value;
    row.append(k, v);
    box.append(row);
  }
  // 有改动才给提交入口；干净文件没有可提交的内容
  if (info.file_status) {
    box.append(commitButton(path, info.identity));
  }
}

function commitButton(path: string, identity?: { name: string; email: string }): HTMLElement {
  const btn = document.createElement("button");
  btn.className = "insp-git-init";
  btn.textContent = "暂存并提交…";
  btn.addEventListener("click", () => openCommitDialog(path, identity));
  return btn;
}

/** Minimal commit dialog (Plan 35): message + optional author fields. */
function openCommitDialog(path: string, identity?: { name: string; email: string }): void {
  if (document.getElementById("commit-dialog")) return;
  const overlay = document.createElement("div");
  overlay.id = "commit-dialog";
  overlay.className = "palette-overlay";
  const box = document.createElement("div");
  box.className = "palette-box commit-box";
  const title = document.createElement("div");
  title.className = "panel-title";
  title.textContent = "暂存并提交当前文件";
  const msg = document.createElement("textarea");
  msg.className = "commit-msg";
  msg.placeholder = "提交信息（必填）";
  const name = document.createElement("input");
  name.type = "text";
  name.className = "commit-field";
  name.placeholder = "作者姓名（留空用 git 配置）";
  name.value = identity?.name ?? "";
  const email = document.createElement("input");
  email.type = "text";
  email.className = "commit-field";
  email.placeholder = "作者邮箱（留空用 git 配置）";
  email.value = identity?.email ?? "";
  const row = document.createElement("div");
  row.className = "commit-actions";
  const cancel = document.createElement("button");
  cancel.className = "insp-git-init";
  cancel.textContent = "取消";
  const submit = document.createElement("button");
  submit.className = "insp-git-init";
  submit.textContent = "提交";
  row.append(cancel, submit);
  box.append(title, msg, name, email, row);
  overlay.append(box);
  document.getElementById("app")?.append(overlay);
  const close = (): void => overlay.remove();
  cancel.addEventListener("click", close);
  overlay.addEventListener("mousedown", (e) => {
    if (e.target === overlay) close();
  });
  submit.addEventListener("click", () => {
    const message = msg.value.trim();
    if (!message) {
      toastMsg("提交信息不能为空");
      return;
    }
    const n = name.value.trim();
    const m = email.value.trim();
    submit.disabled = true;
    void gitCommit(path, message, n || undefined, m || undefined)
      .then((hash) => {
        close();
        toastMsg(`已提交 ${hash}`);
        gitProbeToken++;
        void fillGitSection(path);
      })
      .catch((err) => {
        submit.disabled = false;
        reportError(err);
      });
  });
  msg.focus();
}

let gitProbeToken = 0;

async function initRepoFor(path: string): Promise<void> {
  const dir = path.replace(/[\\/][^\\/]*$/, "") || path;
  const go = await ask(`将在 ${dir} 执行 git init，是否继续？`, {
    title: "初始化仓库",
    kind: "info",
    okLabel: "初始化",
  });
  if (!go) return;
  try {
    const msg = await gitInit(path);
    toastMsg(`已初始化仓库：${msg.split(":").pop()?.trim() || dir}`);
    gitProbeToken++; // invalidate any in-flight probe, then refresh
    await fillGitSection(path);
  } catch (err) {
    reportError(err);
  }
}

function inspSection(title: string): HTMLElement {
  const el = document.createElement("div");
  el.className = "insp-section";
  el.textContent = title;
  return el;
}

/** Chips + input for one document's tags; add/remove re-renders the block. */
function buildTagBlock(path: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "insp-tags";
  const rerender = (): void => {
    const next = buildTagBlock(path);
    wrap.replaceWith(next);
    next.querySelector("input")?.focus();
  };
  for (const tag of tagStore[path] ?? []) {
    const chip = document.createElement("span");
    chip.className = "insp-tag";
    const label = document.createElement("span");
    label.textContent = tag;
    const x = document.createElement("button");
    x.className = "insp-tag-x";
    x.title = `删除标签 ${tag}`;
    x.textContent = "×";
    x.addEventListener("click", () => {
      tagStore = removeTag(tagStore, path, tag);
      persistTags();
      rerender();
    });
    chip.append(label, x);
    wrap.append(chip);
  }
  const input = document.createElement("input");
  input.type = "text";
  input.className = "insp-tag-input";
  input.placeholder = "加标签，回车确认";
  input.addEventListener("keydown", (e) => {
    if (e.key !== "Enter" || isIMEEvent(e)) return;
    e.preventDefault();
    const r = addTag(tagStore, path, input.value);
    if (r.tag) {
      tagStore = r.store;
      persistTags();
      rerender();
    } else {
      if (r.reason === "dup") toastMsg("标签已存在");
      else if (r.reason === "full") toastMsg(`每个文档最多 ${TAGS_PER_DOC_MAX} 个标签`);
      input.value = "";
    }
  });
  wrap.append(input);
  return wrap;
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
  const blfBtn = mkBtn("打开 BLF…", "打开 Vector BLF 二进制日志（压缩/未压缩）", () => void openBlfFile());
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
    blfBtn,
    csvBtn,
    jsonlBtn,
    canExportCancelBtn,
    canExportStatus,
  );

  dock.append(bar, buildCanFilterRow(), (canHostEl = mkHost()));
  canDockEl = dock;
  return dock;
}

// ---- MF4 measurement dock (Roadmap-4 increment 1) ---------------------------

let mf4DockEl: HTMLElement | null = null;
let mf4View: Mf4View | null = null;
let mf4SessionId: number | null = null;
let mf4Path = "";
let mf4Token = 0;
let mf4Opening = false;
let mf4ExportReq: number | null = null;
let mf4SeriesToken = 0;

function buildMf4Dock(): HTMLElement {
  if (mf4DockEl) return mf4DockEl;
  const dock = document.createElement("div");
  dock.id = "mf4-dock";
  dock.className = "mf4-dock";
  dock.style.display = "none";
  mf4DockEl = dock;
  return dock;
}

async function openMf4File(): Promise<void> {
  const picked = await open({
    multiple: false,
    filters: [{ name: "MF4 测量文件", extensions: ["mf4"] }],
  });
  if (typeof picked !== "string") return;
  if (mf4Opening) return;
  mf4Opening = true;
  let opened: number | null = null;
  try {
    const info = await mf4Open(picked);
    opened = info.id;
    const [channels, summary] = await Promise.all([
      mf4Channels({ id: info.id, limit: 2000 }),
      mf4Summary(info.id),
    ]);
    if (mf4SessionId !== null) void mf4Close(mf4SessionId).catch(() => {});
    mf4SessionId = info.id;
    opened = null; // the session now belongs to mf4SessionId/closeMf4
    mf4Path = picked;
    if (canDockVisible) toggleCanDock();
    if (parserDockVisible) toggleParserDock();
    if (viewerDockVisible) toggleViewerDock();
    const host = buildMf4Dock();
    host.style.display = "";
    mf4View = new Mf4View(host, {
      info,
      summary,
      channels: channels.items,
      hasTimeAxis: channels.items.some((c) => c.master),
      onQuery: (q) => void runMf4Query(q),
      onSeriesQuery: (q) => void runMf4Series(info.id, q),
      onExport: (a) => void exportMf4Csv(info.id, a),
      onCancelExport: () => {
        if (mf4ExportReq !== null) {
          void mf4ExportCancel(mf4ExportReq).catch(() => {});
          mf4View?.setStatus("取消导出…");
        }
      },
      onClose: () => closeMf4(),
    });
    const loaded = channels.items.length;
    mf4View.setStatus(
      channels.total > loaded
        ? `共 ${channels.total} 个通道，仅加载前 ${loaded}，请用搜索缩小范围`
        : `已打开 ${basenameOf(picked)}`,
    );
  } catch (err) {
    // A failed follow-up call must not burn one of the four session slots.
    if (opened !== null) void mf4Close(opened).catch(() => {});
    reportError(err);
  } finally {
    mf4Opening = false;
  }
}

/** Fetch one page; a newer query invalidates an in-flight one (later wins). */
async function runMf4Query(q: Mf4Query): Promise<void> {
  if (mf4SessionId === null || !mf4View) return;
  const token = ++mf4Token;
  mf4View.setBusy(true);
  mf4View.setStatus("读取中…");
  try {
    const rows = await mf4Records({ id: mf4SessionId, ...q });
    if (token !== mf4Token) return;
    mf4View.setRows(rows);
    mf4View.setStatus(rows.timeMs.length === 0 ? "该区间无样本" : "");
  } catch (err) {
    if (token !== mf4Token) return;
    reportError(err);
    mf4View.setStatus("读取失败");
  } finally {
    if (token === mf4Token) mf4View?.setBusy(false);
  }
}

/** Fetch the waveform envelope; a newer query invalidates an in-flight one. */
async function runMf4Series(
  id: number,
  q: { channels: number[]; fromSec: number | null; toSec: number | null; buckets: number },
): Promise<void> {
  if (!mf4View) return;
  const token = ++mf4SeriesToken;
  mf4View.setBusy(true);
  try {
    const data = await mf4Series({
      id,
      channels: q.channels,
      fromSec: q.fromSec,
      toSec: q.toSec,
      buckets: q.buckets,
    });
    if (token !== mf4SeriesToken) return;
    mf4View.setSeries(data);
    mf4View.setStatus(data.hasTimeAxis ? "" : "该文件无时间轴，无法绘制波形");
  } catch (err) {
    if (token !== mf4SeriesToken) return;
    reportError(err);
    mf4View.setStatus("波形读取失败");
  } finally {
    if (token === mf4SeriesToken) mf4View?.setBusy(false);
  }
}

async function exportMf4Csv(
  id: number,
  a: { channels: number[]; fromSec: number | null; toSec: number | null },
): Promise<void> {
  const dest = await save({
    defaultPath: `${basenameOf(mf4Path).replace(/\.mf4$/i, "")}.csv`,
    filters: [{ name: "CSV", extensions: ["csv"] }],
  });
  if (typeof dest !== "string") return;
  mf4View?.setBusy(true);
  mf4View?.setStatus("导出中…");
  mf4View?.setExporting(true);
  try {
    mf4ExportReq = await mf4ExportCsv({
      id,
      channels: a.channels,
      fromSec: a.fromSec,
      toSec: a.toSec,
      targetPath: dest,
      onProgress: (p) => mf4View?.setStatus(`导出 ${p.written} 行…`),
      onDone: (s) => {
        mf4ExportReq = null;
        mf4View?.setExporting(false);
        mf4View?.setStatus(
          s.errored ? "导出失败" : s.cancelled ? "已取消导出" : `已导出 ${s.written} 行`,
        );
        mf4View?.setBusy(false);
      },
    });
  } catch (err) {
    mf4ExportReq = null;
    mf4View?.setExporting(false);
    reportError(err);
    mf4View?.setBusy(false);
    mf4View?.setStatus("导出失败");
  }
}

function closeMf4(): void {
  mf4Token++;
  mf4SeriesToken++;
  mf4View?.destroy();
  if (mf4ExportReq !== null) {
    void mf4ExportCancel(mf4ExportReq).catch(() => {});
    mf4ExportReq = null;
  }
  if (mf4SessionId !== null) {
    void mf4Close(mf4SessionId).catch(() => {});
    mf4SessionId = null;
  }
  mf4View = null;
  if (mf4DockEl) {
    mf4DockEl.style.display = "none";
    mf4DockEl.textContent = "";
  }
}

// ---- parser plugin dock (plugin interface, increment 2) ---------------------

let parserDockEl: HTMLElement | null = null;
let parserDockVisible = false;
let parserPanel: ParserPanel | null = null;
let parserHostEl: HTMLElement | null = null;
let parserSel: HTMLSelectElement | null = null;
let parserRunBtn: HTMLButtonElement | null = null;
let parserConsoleEl: HTMLElement | null = null;
let parserStore: RowStore | null = null;
let parserList: ParserPluginInfo[] = [];
let parserReqId: number | null = null;
let parserToken = 0;

function buildParserDock(): HTMLElement {
  if (parserDockEl) return parserDockEl;
  const dock = document.createElement("div");
  dock.id = "parser-dock";
  dock.className = "can-dock";
  dock.style.display = "none";

  const bar = document.createElement("div");
  bar.className = "can-dockbar";
  parserSel = document.createElement("select");
  parserSel.className = "can-src";
  parserSel.addEventListener("change", () => applyParserColumns());
  parserRunBtn = mkBtn("解析", "用选中的解析插件解析当前文档", () => void onParserRunClick());
  const reloadBtn = mkBtn("重载插件", "重新扫描 plugins 目录", () => void loadParserPlugins(true));
  const clearBtn = mkBtn("清空", "清空解析结果", () => {
    parserStore?.clear();
    parserPanel?.render();
    parserPanel?.setStatus("已清空");
  });
  bar.append(document.createTextNode("插件 "), parserSel, parserRunBtn, reloadBtn, clearBtn);

  const consoleBox = document.createElement("div");
  consoleBox.className = "parser-console";
  consoleBox.style.display = "none";
  parserConsoleEl = consoleBox;

  dock.append(bar, (parserHostEl = mkHost()), consoleBox);
  parserDockEl = dock;
  return dock;
}

function toggleParserDock(): void {
  parserDockVisible = !parserDockVisible;
  if (parserDockEl) parserDockEl.style.display = parserDockVisible ? "" : "none";
  if (parserDockVisible) {
    // One bottom dock at a time (CanPanel/MF4 同款互斥).
    if (mf4SessionId !== null) closeMf4();
    if (canDockVisible) toggleCanDock();
    if (viewerDockVisible) toggleViewerDock();
    ensureParserPanel();
    void loadParserPlugins(false);
  }
  if (!parserDockVisible && parserReqId !== null) {
    stopParserRun(); // hiding the dock stops the run (results are ephemeral)
  }
}

function ensureParserPanel(): void {
  if (parserPanel || !parserHostEl) return;
  parserPanel = new ParserPanel(parserHostEl, {
    onRowActivate: (lineNumber) => revealSourceLine(lineNumber),
    onError: (e) => toastMsg(e),
  });
}

/** Refresh the plugin dropdown; preselects the plugin matching the active
 *  document's extension. `report` surfaces load errors as a toast. */
async function loadParserPlugins(report: boolean): Promise<void> {
  const token = ++parserToken;
  try {
    const reply = await parserPluginsList();
    if (token !== parserToken) return;
    for (const e of reply.errors) console.warn(e);
    if (reply.errors.length > 0 && report) {
      toastMsg(`${reply.errors.length} 个解析插件加载失败（详情见控制台）`);
    }
    parserList = reply.parsers;
    const sel = parserSel;
    if (!sel) return;
    sel.textContent = "";
    for (const p of reply.parsers) {
      const o = document.createElement("option");
      o.value = p.id;
      o.textContent = `${p.name}（${p.columns.length} 列）`;
      sel.append(o);
    }
    // Preselect by the active doc's extension, else the first plugin.
    const a = tabs.active();
    const ds = a ? docStates.get(a.info.docId) : undefined;
    const ext = ds ? extOf(ds.path) : "";
    const match = reply.parsers.find((p) =>
      p.extensions.some((e) => e.toLowerCase() === ext),
    );
    if (match) sel.value = match.id;
    applyParserColumns();
    if (reply.parsers.length === 0) parserPanel?.setStatus("plugins 目录没有可用的解析插件");
  } catch (err) {
    reportError(err);
  }
}

/** Swap the column set for the selected plugin; rows from other plugins are
 *  dropped (different shape). */
function applyParserColumns(): void {
  const id = parserSel?.value;
  const plugin = parserList.find((p) => p.id === id);
  if (!plugin) {
    parserPanel?.setColumns([]);
    return;
  }
  parserStore = new RowStore();
  parserPanel?.attachStore(parserStore);
  parserPanel?.setColumns(plugin.columns);
  parserPanel?.render();
  parserPanel?.setStatus(`已选插件：${plugin.name}，点「解析」开始`);
}

async function onParserRunClick(): Promise<void> {
  if (parserReqId !== null) {
    stopParserRun();
    return;
  }
  const a = tabs.active();
  if (!a) {
    toastMsg("请先打开一个文档");
    return;
  }
  const pluginId = parserSel?.value;
  if (!pluginId) {
    toastMsg("没有可用的解析插件");
    return;
  }
  const plugin = parserList.find((p) => p.id === pluginId);
  if (!plugin) return;
  parserStore = new RowStore();
  parserPanel?.attachStore(parserStore);
  parserPanel?.setColumns(plugin.columns);
  parserPanel?.setStatus("解析中…");
  setParserBusy(true);
  if (parserConsoleEl) parserConsoleEl.textContent = "";
  const token = ++parserToken;
  try {
    parserReqId = await parserRun({
      docId: a.info.docId,
      pluginId,
      onBatch: (rows) => {
        if (token !== parserToken) return; // dock closed / plugin switched
        parserStore?.push(rows);
        parserPanel?.render();
      },
      onConsole: (m) => appendParserConsole(m),
      onDone: (s) => {
        if (token !== parserToken) return;
        parserReqId = null;
        setParserBusy(false);
        parserPanel?.setStatus(
          s.error
            ? `解析失败：${s.error}`
            : s.cancelled
              ? "已取消"
              : `已解析 ${s.rows} 行 · 跳过 ${s.skipped} 行 · 共扫描 ${s.lines} 行`,
        );
      },
    });
  } catch (err) {
    parserReqId = null;
    setParserBusy(false);
    reportError(err);
  }
}

function stopParserRun(): void {
  if (parserReqId !== null) void parserCancel(parserReqId).catch(() => {});
}

function setParserBusy(busy: boolean): void {
  if (parserRunBtn) parserRunBtn.textContent = busy ? "停止" : "解析";
}

function appendParserConsole(m: { level: string; text: string }): void {
  if (!parserConsoleEl) return;
  parserConsoleEl.style.display = "";
  const line = document.createElement("div");
  line.textContent = `[${m.level}] ${m.text}`;
  parserConsoleEl.append(line);
  // Same 200-line cap as the script console.
  while (parserConsoleEl.childElementCount > 200) parserConsoleEl.firstElementChild?.remove();
  parserConsoleEl.scrollTop = parserConsoleEl.scrollHeight;
}

// ---- document viewer dock (doc-viewer increment 1) ---------------------------

let viewerDockEl: HTMLElement | null = null;
let viewerDockVisible = false;
let viewerPanel: ParserPanel | null = null;
let viewerHostEl: HTMLElement | null = null;
let viewerSheetSel: HTMLSelectElement | null = null;
let viewerMoreBtn: HTMLButtonElement | null = null;
let viewerInfo: ViewerInfo | null = null;
let viewerStore: RowStore | null = null;
let viewerLoaded = 0;
let viewerToken = 0;
// pdf 两态（Plan 22）：kind==="pdf" 时表格路径整体让位给 canvas 渲染。
let viewerPdfBar: HTMLElement | null = null;
let viewerPdfHost: HTMLElement | null = null;
let viewerPdf: PdfView | null = null;
let viewerPdfPage = 1;
let viewerPdfTotal = 0;
let viewerPdfZoom = 1;
let viewerPdfBaseWidth: number | null = null;
let viewerPdfPrev: HTMLButtonElement | null = null;
let viewerPdfNext: HTMLButtonElement | null = null;
let viewerPdfPageInput: HTMLInputElement | null = null;
let viewerPdfPageLabel: HTMLElement | null = null;
let viewerPdfZoomSel: HTMLSelectElement | null = null;
let viewerExportBtn: HTMLButtonElement | null = null;
let viewerPdfSearchInput: HTMLInputElement | null = null;
let viewerPdfSearchLabel: HTMLElement | null = null;
let viewerPdfSearchIndex = 0;
let viewerPdfLastQuery: string | null = null;
let viewerThumbsRail: HTMLElement | null = null;
let viewerThumbsOn = true;

const VIEWER_WINDOW = 1000;
const PDF_ZOOMS = ["0.5", "0.75", "1", "1.25", "1.5", "2", "3", "4"];

function buildViewerDock(): HTMLElement {
  if (viewerDockEl) return viewerDockEl;
  const dock = document.createElement("div");
  dock.id = "viewer-dock";
  dock.className = "can-dock";
  dock.style.display = "none";

  const bar = document.createElement("div");
  bar.className = "can-dockbar";
  viewerSheetSel = document.createElement("select");
  viewerSheetSel.className = "can-src";
  viewerSheetSel.addEventListener("change", () => void loadViewerRows(true));
  viewerMoreBtn = mkBtn("加载后续 1000 行", "继续取下一页行", () => void loadViewerRows(false));
  viewerMoreBtn.style.display = "none";
  viewerExportBtn = mkBtn("导出 CSV", "把当前工作表导出为 CSV（docx/pdf 为 TXT）", () => void exportViewer());
  bar.append(document.createTextNode("工作表 "), viewerSheetSel, viewerMoreBtn, viewerExportBtn!);

  // pdf 工具条：默认隐藏，kind==="pdf" 时替换表格控件。
  viewerPdfBar = document.createElement("div");
  viewerPdfBar.className = "can-dockbar";
  viewerPdfBar.style.display = "none";
  viewerPdfPrev = mkBtn("◀", "上一页", () => {
    viewerPdfPage = clampPage(viewerPdfPage - 1, viewerPdfTotal);
    void renderViewerPdf();
  });
  viewerPdfNext = mkBtn("▶", "下一页", () => {
    viewerPdfPage = clampPage(viewerPdfPage + 1, viewerPdfTotal);
    void renderViewerPdf();
  });
  viewerPdfPageInput = document.createElement("input");
  viewerPdfPageInput.type = "text";
  viewerPdfPageInput.className = "can-fnum";
  viewerPdfPageInput.style.width = "56px";
  viewerPdfPageInput.addEventListener("change", () => {
    viewerPdfPage = clampPage(Number(viewerPdfPageInput!.value), viewerPdfTotal);
    void renderViewerPdf();
  });
  viewerPdfPageLabel = document.createElement("span");
  viewerPdfPageLabel.className = "can-status";
  viewerPdfZoomSel = document.createElement("select");
  viewerPdfZoomSel.className = "can-src";
  for (const z of PDF_ZOOMS) {
    const o = document.createElement("option");
    o.value = z;
    o.textContent = `${Math.round(Number(z) * 100)}%`;
    viewerPdfZoomSel.append(o);
  }
  viewerPdfZoomSel.value = "1";
  viewerPdfZoomSel.addEventListener("change", () => {
    viewerPdfZoom = Number(viewerPdfZoomSel!.value);
    void renderViewerPdf();
  });
  const fitBtn = mkBtn("适应宽度", "缩放到当前面板宽度", () => {
    if (viewerPdfBaseWidth && viewerPdfHost) {
      viewerPdfZoom = Math.max(0.5, Math.min(4, (viewerPdfHost.clientWidth - 24) / viewerPdfBaseWidth));
      viewerPdfZoomSel!.value = String(viewerPdfZoom);
    }
    void renderViewerPdf();
  });
  const zoomIn = mkBtn("＋", "放大（×1.25）", () => {
    viewerPdfZoom = stepZoom(viewerPdfZoom, "in");
    viewerPdfZoomSel!.value = String(viewerPdfZoom);
    void renderViewerPdf();
  });
  const zoomOut = mkBtn("－", "缩小（×0.8）", () => {
    viewerPdfZoom = stepZoom(viewerPdfZoom, "out");
    viewerPdfZoomSel!.value = String(viewerPdfZoom);
    void renderViewerPdf();
  });
  // 文本搜索（Plan 24）：Enter 全文档搜索并跳到第一条，上下条循环，Esc 清除。
  viewerPdfSearchInput = document.createElement("input");
  viewerPdfSearchInput.type = "text";
  viewerPdfSearchInput.className = "can-ftxt";
  viewerPdfSearchInput.placeholder = "在 PDF 中查找";
  viewerPdfSearchInput.addEventListener("keydown", (e) => {
    if (isIMEEvent(e)) return;
    if (e.key === "Enter") {
      void onPdfSearch(e.shiftKey ? "prev" : "next");
    } else if (e.key === "Escape") {
      clearPdfSearch();
    }
  });
  viewerPdfSearchLabel = document.createElement("span");
  viewerPdfSearchLabel.className = "can-status";
  const searchPrev = mkBtn("◀", "上一处命中", () => void onPdfSearch("prev"));
  const searchNext = mkBtn("▶", "下一处命中", () => void onPdfSearch("next"));
  const searchClear = mkBtn("✕", "清除搜索高亮", () => clearPdfSearch());
  const thumbsBtn = mkBtn("缩略图", "显示 / 隐藏页面缩略图", () => {
    viewerThumbsOn = !viewerThumbsOn;
    if (viewerThumbsRail) viewerThumbsRail.style.display = viewerThumbsOn ? "" : "none";
  });
  viewerPdfBar.append(
    viewerPdfPrev!,
    viewerPdfPageInput!,
    viewerPdfPageLabel!,
    viewerPdfNext!,
    document.createTextNode(" 缩放 "),
    zoomOut,
    viewerPdfZoomSel!,
    zoomIn,
    fitBtn,
    document.createTextNode(" 查找 "),
    viewerPdfSearchInput!,
    searchPrev,
    searchNext,
    searchClear,
    viewerPdfSearchLabel!,
    thumbsBtn,
  );

  const pdfHost = document.createElement("div");
  pdfHost.className = "pdf-host";
  viewerPdfHost = pdfHost;
  viewerThumbsRail = document.createElement("div");
  viewerThumbsRail.className = "pdf-thumbs";
  viewerThumbsRail.style.display = "none";
  // 缩略图栏与渲染区横向并列（dock 本体是纵向 flex）。
  const pdfBody = document.createElement("div");
  pdfBody.className = "pdf-body";
  pdfBody.style.display = "none";
  pdfBody.append(viewerThumbsRail, pdfHost);

  dock.append(bar, viewerPdfBar, (viewerHostEl = mkHost()), pdfBody);
  viewerDockEl = dock;
  return dock;
}

function toggleViewerDock(): void {
  viewerDockVisible = !viewerDockVisible;
  if (viewerDockEl) viewerDockEl.style.display = viewerDockVisible ? "" : "none";
  if (viewerDockVisible) {
    // One bottom dock at a time.
    if (mf4SessionId !== null) closeMf4();
    if (canDockVisible) toggleCanDock();
    if (parserDockVisible) toggleParserDock();
    ensureViewerPanel();
  }
  if (!viewerDockVisible && viewerInfo !== null) {
    void viewerClose(viewerInfo.id).catch(() => {});
    viewerInfo = null;
    viewerPdf?.destroy();
    viewerPdf = null;
    viewerPdfTotal = 0;
  }
}

function ensureViewerPanel(): void {
  if (viewerPanel || !viewerHostEl) return;
  viewerPanel = new ParserPanel(viewerHostEl, {
    // xlsx 行号可跳转没有任何意义（本文件不是文本源），不接线。
    onRowActivate: () => {},
    onError: (e) => toastMsg(e),
  });
}

async function openViewerFile(): Promise<void> {
  const picked = await open({
    multiple: false,
    filters: [
      {
        name: "文档（xlsx/xlsm/docx/pdf/pptx）",
        extensions: ["xlsx", "xlsm", "docx", "pdf", "pptx"],
      },
    ],
  });
  if (typeof picked !== "string") return;
  try {
    const info = await viewerOpen(picked);
    if (viewerInfo !== null) void viewerClose(viewerInfo.id).catch(() => {});
    viewerInfo = info;
    if (!viewerDockVisible) toggleViewerDock();
    setViewerMode(info.kind === "pdf" ? "pdf" : "table");
    if (info.kind === "pdf") {
      // 渲染在前端：后端只出字节。换文档即重置搜索状态。
      clearPdfSearch();
      const bytes = await viewerPdfBytes(info.id);
      viewerPdfTotal = (await viewerPdf?.load(bytes)) ?? 0;
      viewerPdfPage = 1;
      viewerPdfBaseWidth = null;
      if (viewerPdfSearchInput) viewerPdfSearchInput.value = "";
      viewerPdf?.initThumbs(viewerThumbsRail!, (p) => {
        viewerPdfPage = p;
        void renderViewerPdf();
      });
      viewerThumbsRail!.style.display = viewerThumbsOn ? "" : "none";
      await renderViewerPdf();
      return;
    }
    const sel = viewerSheetSel;
    if (sel) {
      sel.textContent = "";
      for (const s of info.sheets) {
        const o = document.createElement("option");
        o.value = s;
        o.textContent = s;
        sel.append(o);
      }
    }
    await loadViewerRows(true);
  } catch (err) {
    reportError(err);
  }
}

/** Switch the viewer dock between the table path (xlsx/docx) and pdf render. */
function setViewerMode(mode: "table" | "pdf"): void {
  const pdf = mode === "pdf";
  if (viewerSheetSel) viewerSheetSel.style.display = pdf ? "none" : "";
  if (viewerMoreBtn) viewerMoreBtn.style.display = "none";
  if (viewerHostEl) viewerHostEl.style.display = pdf ? "none" : "";
  if (viewerPdfBar) viewerPdfBar.style.display = pdf ? "" : "none";
  const pdfBody = viewerPdfHost?.parentElement;
  if (pdfBody) pdfBody.style.display = pdf ? "" : "none";
  if (viewerExportBtn) viewerExportBtn.textContent = pdf ? "导出 TXT" : "导出 CSV";
  if (pdf && viewerPanel) {
    // detach the table so its scroll window stops painting behind the canvas
    viewerPanel.setStatus("");
  }
  if (!pdf) {
    viewerPdf?.destroy();
    viewerPdf = null;
    viewerPdfTotal = 0;
  }
}

async function renderViewerPdf(): Promise<void> {
  if (!viewerPdf || viewerPdfTotal === 0) return;
  const page = clampPage(viewerPdfPage, viewerPdfTotal);
  viewerPdfPage = page;
  if (viewerPdfPageInput) viewerPdfPageInput.value = String(page);
  if (viewerPdfPageLabel) viewerPdfPageLabel.textContent = `/ ${viewerPdfTotal} 页`;
  const width = await viewerPdf.render(page, viewerPdfZoom);
  if (width !== null) viewerPdfBaseWidth = width / viewerPdfZoom;
}

/** Full-document search (Enter) / hit cycling (◀▶). A changed query
 *  re-searches; the same query just cycles hits. */
async function onPdfSearch(dir: "next" | "prev"): Promise<void> {
  if (!viewerPdf || viewerPdfTotal === 0) return;
  const query = (viewerPdfSearchInput?.value ?? "").trim();
  if (!query) {
    clearPdfSearch();
    return;
  }
  if (query !== viewerPdfLastQuery) {
    if (viewerPdfSearchLabel) viewerPdfSearchLabel.textContent = "搜索中…";
    const count = await viewerPdf.searchAll(query);
    viewerPdfLastQuery = query;
    viewerPdfSearchIndex = 0;
    if (viewerPdfSearchLabel) {
      viewerPdfSearchLabel.textContent = count === 0 ? "无命中" : `命中 ${count} 处`;
    }
    if (count === 0) return;
  }
  viewerPdfSearchIndex =
    dir === "next"
      ? (viewerPdfSearchIndex + 1) % viewerPdf.hitCount
      : (viewerPdfSearchIndex - 1 + viewerPdf.hitCount) % viewerPdf.hitCount;
  await viewerPdf.gotoHit(viewerPdfSearchIndex);
}

function clearPdfSearch(): void {
  viewerPdf?.clearSearch();
  viewerPdfLastQuery = null;
  viewerPdfSearchIndex = 0;
  if (viewerPdfSearchLabel) viewerPdfSearchLabel.textContent = "";
}

/** Pull one window of rows; `reset` restarts from row 1 (sheet switch/open). */
async function loadViewerRows(reset: boolean): Promise<void> {
  if (!viewerInfo) return;
  const token = ++viewerToken;
  try {
    const page = await viewerRows({
      id: viewerInfo.id,
      sheetIndex: viewerSheetSel?.selectedIndex ?? 0,
      fromRow: reset ? 0 : viewerLoaded,
      limit: VIEWER_WINDOW,
    });
    if (token !== viewerToken) return; // sheet switched / doc replaced
    if (reset) {
      viewerStore = new RowStore();
      viewerPanel?.attachStore(viewerStore);
    }
    viewerPanel?.setColumns(page.columns.map((label, i) => ({ key: `c${i}`, label })));
    viewerStore?.push(page.rows);
    viewerLoaded = reset ? page.rows.length : viewerLoaded + page.rows.length;
    viewerPanel?.render();
    viewerPanel?.setStatus(
      `${basenameOf(viewerInfo.path)} · ${viewerLoaded}/${page.total} 行` +
        (viewerInfo.truncated ? " · 内容已截断" : ""),
    );
    if (viewerMoreBtn) viewerMoreBtn.style.display = viewerLoaded < page.total ? "" : "none";
  } catch (err) {
    reportError(err);
  }
}

async function exportViewer(): Promise<void> {
  const info = viewerInfo;
  if (!info) return;
  const isCsv = info.kind === "xlsx"; // docx/pdf export as plain text
  const dest = await save({
    defaultPath: basenameOf(info.path) + (isCsv ? ".csv" : ".txt"),
    filters: [isCsv ? { name: "CSV", extensions: ["csv"] } : { name: "文本", extensions: ["txt"] }],
  });
  if (typeof dest !== "string") return;
  try {
    const stats = await viewerExportCsv({
      id: info.id,
      sheetIndex: viewerSheetSel?.selectedIndex ?? 0,
      targetPath: dest,
    });
    toastMsg(`已导出 ${stats.written} 行 → ${dest}`);
  } catch (err) {
    reportError(err);
  }
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
  const key = blfView ? BLF_KEY : tabs.active()?.info.docId;
  if (key === undefined) return;
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
  canFilters.set(key, f);
  canPanel?.setFilter(f);
}

function toggleCanDock(): void {
  canDockVisible = !canDockVisible;
  if (canDockEl) canDockEl.style.display = canDockVisible ? "" : "none";
  if (canDockVisible) {
    if (mf4SessionId !== null) closeMf4(); // one bottom dock at a time
    if (parserDockVisible) toggleParserDock();
    if (viewerDockVisible) toggleViewerDock();
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
  if (blfView) {
    canPanel.attachStore(blfView.store);
    const f = canFilters.get(BLF_KEY) ?? {};
    canPanel.setFilter(f);
    restoreFilterInputs(f);
    if (canRunBtn && !canParsing) canRunBtn.textContent = "重新解析 BLF";
    return;
  }
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
  if (canRunBtn && !canParsing) canRunBtn.textContent = "解析报文";
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
  if (blfView) {
    toastMsg("BLF 无源文本，无法跳转到行");
    return;
  }
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

/** Pick a .blf and stream it into the dock (replacing any previous BLF table). */
async function openBlfFile(): Promise<void> {
  const picked = await open({
    multiple: false,
    filters: [{ name: "BLF", extensions: ["blf"] }],
  });
  if (typeof picked !== "string") return;
  await startBlfParse(picked);
}

async function startBlfParse(path: string): Promise<void> {
  if (canParsing || canExportBusy) {
    toastMsg(canParsing ? "解析进行中，请稍候…" : "导出进行中，请稍候…");
    return;
  }
  if (!canDockVisible) toggleCanDock();
  ensurePanel();
  const store = new FrameStore();
  blfView = { path, store };
  store.clear();
  canPanel!.attachStore(store);
  const f = canFilters.get(BLF_KEY) ?? {};
  canPanel!.setFilter(f);
  restoreFilterInputs(f);
  setCanParsing(true);
  canReqId = null;
  try {
    canReqId = await canParseBlf({
      path,
      dbcId: lastDbcId,
      onBatch: (frames) => {
        store.push(frames);
        canPanel?.render();
      },
      onDone: (s) => {
        toastMsg(`BLF 解码 ${s.decoded} 帧，跳过 ${s.skipped} 个对象`);
        if (s.error) toastMsg(`BLF 解析中断：${s.error}`);
        canPanel?.render();
        setCanParsing(false);
      },
    });
    if (canParsing && canRunBtn) canRunBtn.textContent = "停止";
  } catch (err) {
    blfView = null;
    reportError(err);
    setCanParsing(false);
    switchCanView(); // put the previous document's table back
  }
}

function onCanRunClick(): void {
  if (blfView) {
    if (canParsing) {
      if (canReqId !== null) canCancel(canReqId).catch((e) => reportError(e));
    } else {
      void startBlfParse(blfView.path);
    }
    return;
  }
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

/** Export the active view's CAN frames (document re-decode or BLF, spec §7.5). */
async function onCanExport(format: "csv" | "jsonl"): Promise<void> {
  const blf = blfView;
  const a = tabs.active();
  if (!blf && !a) {
    toastMsg("请先打开一个文档");
    return;
  }
  if (canExportBusy || canParsing) {
    toastMsg("CAN 任务进行中，请稍候…");
    return;
  }
  const docId = a ? a.info.docId : null;
  const ds = docId !== null ? docStates.get(docId) : undefined;
  const base = blf ? basenameOf(blf.path) : ds ? basenameOf(ds.path) : "can";
  const path = await save({
    defaultPath: `${base}.${format}`,
    filters: [{ name: format === "csv" ? "CSV" : "JSONL", extensions: [format] }],
  });
  if (typeof path !== "string") return;
  const filter = canFilters.get(blf ? BLF_KEY : docId!) ?? {};
  updateCanExportUi(true);
  canExportReqId = null;
  if (canExportStatus) canExportStatus.textContent = "导出中…";
  const onProgress = (p: { written: number }): void => {
    if (canExportStatus) canExportStatus.textContent = `导出中… ${p.written} 帧`;
  };
  const onDone = (s: CanExportStats): void => {
    updateCanExportUi(false);
    const msg = s.errored
      ? `导出中断（已写 ${s.written} 帧）`
      : s.cancelled
        ? `已取消，导出 ${s.written} 帧`
        : `已导出 ${s.written} 帧 → ${basenameOf(path)}`;
    if (canExportStatus) canExportStatus.textContent = msg;
    toastMsg(msg);
  };
  try {
    if (blf) {
      const args = { path: blf.path, dbcId: lastDbcId, filter, targetPath: path, onProgress, onDone };
      canExportReqId = format === "csv" ? await exportBlfCsv(args) : await exportBlfJsonl(args);
    } else {
      const sourceKind = canSrcSel!.value;
      const regex = sourceKind === "regex" ? canRegexInput!.value : null;
      const args = {
        docId: docId!,
        dbcId: lastDbcId,
        sourceKind,
        regex,
        filter,
        targetPath: path,
        onProgress,
        onDone,
      };
      canExportReqId = format === "csv" ? await exportCsv(args) : await exportJsonl(args);
    }
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
  if (canRunBtn) {
    canRunBtn.textContent = b ? "解析中…" : blfView ? "重新解析 BLF" : "解析报文";
  }
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

async function openPath(path: string, opts?: { quiet?: boolean }): Promise<boolean> {
  if (busy) return false;
  setBusy(true);
  try {
    const info = await docOpen(path, onIndexProgress);
    // The shell owns the ≤200MB split (spec §4.3/A1) via `DocInfo.mode`.
    if (info.mode === "readonly") mountList(info, path);
    else await mountDoc(info, path);
    void refreshWatchStatus(); // a new directory may have failed to watch
    void refreshRecent(); // the shell recorded the open; palette list follows
    saveSession();
    return true;
  } catch (err) {
    if (!opts?.quiet) reportError(err); // startup restore skips missing files
    return false;
  } finally {
    setBusy(false);
  }
}

/** Index progress tick (Plan 37): non-done ticks move the 待索引 boundary and
 *  refresh the active list (throttled); the done tick corrects the real total. */
let lastIndexRefresh = 0;
function onIndexProgress(p: IndexProgress): void {
  const ds = docStates.get(p.docId);
  if (!ds) return;
  indexed.set(p.docId, p.indexedLines);
  const list = lists.get(p.docId);
  if (!list) return;
  if (p.done) {
    ds.totalLines = p.indexedLines;
    if (p.docId === tabs.active()?.info.docId) {
      status.setReadonly(p.indexedLines);
      updateInspector(p.docId);
    }
    list.refresh();
    return;
  }
  const now = Date.now();
  if (p.docId === tabs.active()?.info.docId && now - lastIndexRefresh >= 120) {
    lastIndexRefresh = now;
    list.refresh();
  }
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

// ---- editor parity commands (Plan 27: line ops / wrap / font zoom) ----------

let editorWrapOn = loadFlag("lumelog.ui.wrap", true);

// ---- saved macros (Plan 30: cross-session persistence) -----------------------

const MACRO_KEY = "lumelog.macros.v1";
let savedMacros: NamedMacro[] = (() => {
  try {
    return parseMacros(localStorage.getItem(MACRO_KEY));
  } catch {
    return [];
  }
})();

function persistMacros(): void {
  try {
    localStorage.setItem(MACRO_KEY, serializeMacros(savedMacros));
  } catch {
    /* storage unavailable/over quota: keep the session copy, toast on save */
  }
}

// ---- document tags (Plan 33: inspector, keyed by path) -----------------------

const TAG_KEY = "lumelog.doc-tags.v1";
let tagStore: TagStore = (() => {
  try {
    return parseTagStore(localStorage.getItem(TAG_KEY));
  } catch {
    return {};
  }
})();

// ---- macro keyboard bindings (Plan 41) ---------------------------------------

const MACRO_BIND_KEY = "lumelog.keymap.macros.v1";
let macroBindings: MacroBindings = (() => {
  try {
    return parseMacroBindings(localStorage.getItem(MACRO_BIND_KEY));
  } catch {
    return {};
  }
})();

function persistMacroBindings(): void {
  try {
    localStorage.setItem(MACRO_BIND_KEY, serializeMacroBindings(macroBindings));
  } catch {
    /* storage unavailable: bindings stay session-only */
  }
}

function persistTags(): void {
  try {
    localStorage.setItem(TAG_KEY, serializeTagStore(tagStore));
  } catch {
    /* storage unavailable: session-only tags */
  }
}

// ---- workspace session restore (Plan 40) -------------------------------------

const SESSION_KEY = "lumelog.session.v1";

/** Snapshot the open tabs (order + active path) to localStorage. */
function saveSession(): void {
  const session = normalizeSession({
    paths: tabs.list().map((t) => t.path),
    active: tabs.active()?.info.path ?? null,
  });
  try {
    localStorage.setItem(SESSION_KEY, serializeSession(session));
  } catch {
    /* storage unavailable: no session restore */
  }
}

/** Reopen last session's tabs at startup; missing files skip silently. */
async function restoreSession(): Promise<void> {
  const session = parseSession(localStorage.getItem(SESSION_KEY));
  if (session.paths.length === 0) return;
  let opened = 0;
  let missing = 0;
  let activeDocId: number | null = null;
  for (const p of session.paths) {
    if (await openPath(p, { quiet: true })) {
      opened++;
      const entry = tabs.list().find((t) => t.path === p);
      if (p === session.active && entry) activeDocId = entry.docId;
    } else {
      missing++;
    }
  }
  if (opened === 0 && missing === 0) return;
  const parts: string[] = [];
  if (opened > 0) parts.push(`已恢复 ${opened} 个文档`);
  if (missing > 0) parts.push(`${missing} 个文件已缺失，已跳过`);
  toastMsg(parts.join("；"));
  if (activeDocId !== null) tabs.activate(activeDocId);
}

// ---- bookmarks across sessions (Plan 38, per file path) ----------------------

const BOOKMARK_KEY = "lumelog.bookmarks.v1";
let bookmarkStore: BookmarkStore = (() => {
  try {
    return parseBookmarkStore(localStorage.getItem(BOOKMARK_KEY));
  } catch {
    return {};
  }
})();

function persistBookmarks(): void {
  try {
    localStorage.setItem(BOOKMARK_KEY, serializeBookmarkStore(bookmarkStore));
  } catch {
    /* storage unavailable: session-only bookmarks */
  }
}

function activeEditor() {
  const a = tabs.active();
  return a ? editors.get(a.info.docId) : undefined;
}

function toggleEditorWrap(): void {
  editorWrapOn = !editorWrapOn;
  try {
    localStorage.setItem("lumelog.ui.wrap", editorWrapOn ? "1" : "0");
  } catch {
    /* private mode: session-only */
  }
  activeEditor()?.setWrap(editorWrapOn);
  toastMsg(editorWrapOn ? "自动换行：开" : "自动换行：关");
}

// ---- syntax plugins (plugin interface, increment 1) -------------------------

/** Plugin loads replace the whole registry, so overlapping calls are merged. */
let pluginsInFlight: Promise<void> | null = null;

async function initSyntaxPlugins(): Promise<void> {
  if (pluginsInFlight) return pluginsInFlight;
  pluginsInFlight = (async () => {
    try {
      const reply = await syntaxPluginsList();
      const warnings = registerPluginLanguages(reply.plugins);
      status.setSyntaxModes(syntaxModes());
      const problems = [...warnings, ...reply.errors];
      for (const p of problems) console.warn(p);
      if (problems.length > 0) {
        toastMsg(`${problems.length} 条语法插件问题（详情见控制台）`);
      }
    } catch {
      // Plugin loading is optional; a shell without the command stays usable.
    } finally {
      pluginsInFlight = null;
    }
  })();
  return pluginsInFlight;
}

// ---- recent files + drag-drop open (M1 deferred debt) ------------------------

let recentPaths: string[] = [];

function dropOverlay(): HTMLElement {
  const overlay = document.createElement("div");
  overlay.id = "drop-overlay";
  overlay.textContent = "松开以打开文件";
  overlay.style.display = "none";
  return overlay;
}

/** Load the recent list once at startup; `refreshRecent` re-reads after opens. */
async function initRecent(): Promise<void> {
  try {
    recentPaths = await recentList();
  } catch {
    recentPaths = []; // advisory state — never block startup
  }
}

async function refreshRecent(): Promise<void> {
  try {
    recentPaths = await recentList();
  } catch {
    /* keep the previous list */
  }
}

async function clearRecentList(): Promise<void> {
  try {
    await recentClear();
    recentPaths = [];
    toastMsg("已清除最近文件记录");
  } catch (err) {
    reportError(err);
  }
}

/** Full-window hint while files are dragged over the webview. The overlay
 *  element itself is created in `layout()` (`dropOverlay()`). */
function initDragDrop(): void {
  void (async () => {
    try {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      await getCurrentWebview().onDragDropEvent((ev) => {
        const overlay = document.getElementById("drop-overlay");
        if (!overlay) return;
        // Property narrowing does not survive into the nested async closure —
        // capture the dropped paths up front.
        const dropped = ev.payload.type === "drop" ? ev.payload.paths : null;
        if (ev.payload.type === "enter" || ev.payload.type === "over") {
          overlay.style.display = "flex";
        } else if (dropped !== null) {
          overlay.style.display = "none";
          // Sequential: openPath carries the busy guard; the first failure
          // (unreadable file, cancelled confirm) stops the batch.
          void (async () => {
            for (const p of dropped) {
              const ok = await openPath(p);
              if (!ok) break;
            }
          })();
        } else {
          overlay.style.display = "none"; // leave
        }
      });
    } catch {
      // No Tauri webview (plain vite dev): dragging is not available.
    }
  })();
}

// ---- external modification watcher (spec §4.4/A10, M3 B2) -------------------

async function initWatch(): Promise<void> {
  try {
    const st = await watchSubscribe(handleExternalEvent);
    status.setWatch(st.degraded ? "degraded" : "ok", st.degraded ?? undefined);
  } catch {
    status.setWatch("degraded", "订阅外部监听失败");
  }
}

/** Re-read watcher health (degradation can appear when a new dir fails). */
async function refreshWatchStatus(): Promise<void> {
  try {
    const st = await watchStatus();
    status.setWatch(st.degraded ? "degraded" : "ok", st.degraded ?? undefined);
  } catch {
    // Status is advisory; a failure here must not disturb opening.
  }
}

function handleExternalEvent(e: ExternalEvent): void {
  const hits: number[] = [];
  for (const [id, ds] of docStates) {
    if (ds.path === e.path) hits.push(id);
  }
  for (const docId of hits) {
    if (e.kind === "removed") {
      tabs.setExternalFlag(docId, true);
      toastMsg(`文件已被外部删除：${basenameOf(e.path)}`);
      continue;
    }
    const h = editors.get(docId);
    if (h && h.isDirty()) {
      tabs.setExternalFlag(docId, true);
      showExtBar(docId, e.path);
      continue;
    }
    if (lists.has(docId)) {
      void handleReadonlyAppend(docId, e.path);
      continue;
    }
    void reloadDocFromDisk(docId, e.path);
  }
}

/** Non-blocking bar for a dirty doc whose file changed on disk. */
function showExtBar(docId: number, path: string): void {
  if (extBarDocId !== null) hideExtBar(extBarDocId);
  const box = document.createElement("div");
  box.className = "ext-bar";
  box.setAttribute("role", "status");
  const label = document.createElement("span");
  label.className = "ext-label";
  label.textContent = `文件已被外部修改：${basenameOf(path)}`;
  const reloadBtn = mkBtn("重新加载（丢弃我的修改）", "用磁盘内容覆盖当前编辑", () =>
    void reloadDirtyFromDisk(docId, path),
  );
  const keepBtn = mkBtn("保留我的修改", "黄点保留，保存时再决定", () => hideExtBar(docId));
  box.append(label, reloadBtn, keepBtn);
  document.body.append(box);
  extBarEl = box;
  extBarDocId = docId;
}

function hideExtBar(docId: number): void {
  if (extBarDocId !== docId) return;
  extBarEl?.remove();
  extBarEl = null;
  extBarDocId = null;
}

/** Dirty "重新加载": confirm discarding local edits, then reload from disk. */
async function reloadDirtyFromDisk(docId: number, path: string): Promise<void> {
  const go = await ask("重新加载会丢弃当前未保存的修改，是否继续？", {
    title: "外部修改",
    kind: "warning",
    okLabel: "丢弃并重新加载",
    cancelLabel: "取消",
  });
  if (!go) return;
  hideExtBar(docId);
  await reloadDocFromDisk(docId, path);
}

/** Three-way save-conflict choice (spec §4.4; `ask` is only two buttons). */
function confirmExternalConflict(detail: string): Promise<"reload" | "overwrite" | "cancel"> {
  return new Promise((resolve) => {
    const overlay = document.createElement("div");
    overlay.className = "modal-3way";
    overlay.setAttribute("role", "dialog");
    overlay.setAttribute("aria-modal", "true");
    const box = document.createElement("div");
    box.className = "modal-3way-box";
    const title = document.createElement("div");
    title.className = "modal-3way-title";
    title.textContent = "文件已被外部修改";
    const msg = document.createElement("div");
    msg.className = "modal-3way-msg";
    // Keep the shell's informed detail (disk size + unmappable `?` count).
    msg.textContent = `${detail}\n\n重新加载会丢弃你当前的修改；强制覆盖会写入你的版本。`;
    msg.style.whiteSpace = "pre-wrap";
    const row = document.createElement("div");
    row.className = "modal-3way-actions";
    const done = (choice: "reload" | "overwrite" | "cancel"): void => {
      document.removeEventListener("keydown", onKey);
      overlay.remove();
      resolve(choice);
    };
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") done("cancel");
    };
    row.append(
      mkBtn("重新加载", "丢弃我的修改，载入磁盘内容", () => done("reload")),
      mkBtn("强制覆盖", "用我的修改覆盖磁盘", () => done("overwrite")),
      mkBtn("取消", "返回编辑", () => done("cancel")),
    );
    overlay.addEventListener("click", (e) => {
      if (e.target === overlay) done("cancel");
    });
    box.append(title, msg, row);
    overlay.append(box);
    document.body.append(overlay);
    document.addEventListener("keydown", onKey);
    (row.querySelector("button") as HTMLButtonElement | null)?.focus();
  });
}

/** Reload a clean doc from disk, keeping cursor/scroll; dirty callers confirm first. */
async function reloadDocFromDisk(docId: number, path: string): Promise<void> {
  if (watchReloading.has(docId)) {
    watchPending.add(docId);
    return;
  }
  watchReloading.add(docId);
  const ds = docStates.get(docId);
  const h = editors.get(docId);
  const list = lists.get(docId);
  let scroll = 0;
  let sel: { anchor: number; head: number } | null = null;
  if (h) {
    scroll = h.view.scrollDOM.scrollTop;
    const m = h.view.state.selection.main;
    sel = { anchor: m.anchor, head: m.head };
  } else if (list) {
    scroll = list.scrollTop();
  }
  try {
    const info = await docReload(docId, () => {});
    // The tab may have been closed/re-opened while the reload was in flight.
    if (!ds || docStates.get(docId) !== ds || !tabs.get(docId)) return;
    if (info.mode !== ds.mode) {
      // Grew/shrank across the 200 MB threshold: reopen keeps views truthful.
      // The user already consented on the dirty path; do not ask twice.
      if (h && h.isDirty()) h.markSaved();
      const closed = await handleClose(docId);
      if (!closed) return;
      watchPending.delete(docId); // the tab is gone: never replay onto a dead id
      await openPath(path);
      toastMsg("文件大小变化导致视图模式切换，已重新打开");
      return;
    }
    const hNow = editors.get(docId);
    const listNow = lists.get(docId);
    // The user may have typed during the reload: never overwrite fresh edits.
    if (hNow && hNow.isDirty()) {
      tabs.setExternalFlag(docId, true);
      showExtBar(docId, path);
      return;
    }
    ds.encoding = info.encoding;
    ds.eol = info.eol;
    ds.totalLines = info.totalLines ?? info.lineCountEstimate;
    ds.invalid = 0;
    ds.size = info.size;
    ds.mtime = info.mtimeMillis;
    indexed.set(docId, ds.totalLines);
    if (hNow) {
      const { text, invalidCount } = await loadAllText(docId, ds.totalLines);
      ds.invalid = invalidCount;
      hNow.replaceText(text);
      hNow.markSaved();
      const len = hNow.view.state.doc.length;
      if (sel) {
        hNow.view.dispatch({
          selection: { anchor: Math.min(sel.anchor, len), head: Math.min(sel.head, len) },
        });
      }
      hNow.view.scrollDOM.scrollTop = scroll;
      tabs.refreshDirty(docId);
    } else if (listNow) {
      listNow.invalidate();
      listNow.setView(null, false, false);
      queryHits.delete(docId);
      queryCursor.delete(docId);
      listNow.refresh();
      const st = followStates.get(docId);
      if (st?.on) listNow.scrollToBottom();
      else listNow.setScrollTop(scroll);
    }
    syncStatus(docId);
    updateInspector(docId);
    refreshSidebar();
    tabs.setExternalFlag(docId, false);
    hideExtBar(docId);
    toastMsg(`文件已变化，已重新加载：${basenameOf(path)}`);
  } catch (err) {
    tabs.setExternalFlag(docId, true); // reload failed (deleted/locked): keep the warning
    reportError(err);
  } finally {
    watchReloading.delete(docId);
    if (watchPending.delete(docId)) void reloadDocFromDisk(docId, path);
  }
}

async function mountDoc(info: DocInfo, path: string): Promise<void> {
  followStates.delete(info.docId); // edit mode never follows
  const total = info.totalLines ?? info.lineCountEstimate;
  indexed.set(info.docId, total);
  docStates.set(info.docId, {
    path,
    encoding: info.encoding,
    eol: info.eol || "LF",
    totalLines: total,
    mode: "edit",
    invalid: 0,
    size: info.size,
    mtime: info.mtimeMillis,
  });
  const handle = await openEditable({
    parent: tabsEl,
    info,
    ext: extOf(path),
    wrap: editorWrapOn,
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
    // Plan 38: bookmarks survive document close, keyed by file path.
    onBookmarksChanged: (lines) => {
      if (lines.length === 0) delete bookmarkStore[path];
      else bookmarkStore[path] = lines;
      persistBookmarks();
    },
  });
  handle.loadBookmarkLines(bookmarkStore[path] ?? []);
  editors.set(info.docId, handle);
  tabs.add({ docId: info.docId, path }, handle);
  trackCursor(handle);
  syncStatus(info.docId);
}

/** Mount the read-only virtual list (spec §4.3). No full load — windows on demand. */
function mountList(info: DocInfo, path: string): void {
  const total = info.totalLines ?? info.lineCountEstimate;
  // Deferred open (Plan 37): totalLines is null until the done tick — start
  // with nothing indexed so rows render as 待索引 while offsets stream in.
  indexed.set(info.docId, info.totalLines ?? 0);
  docStates.set(info.docId, {
    path,
    encoding: info.encoding,
    eol: info.eol || "LF",
    totalLines: total,
    mode: "readonly",
    invalid: 0,
    size: info.size,
    mtime: info.mtimeMillis,
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
    onScrollTop: (top, maxScroll) => onListScroll(info.docId, top, maxScroll),
  });
  followStates.set(info.docId, { on: false, paused: false });
  tailSeenSize.set(info.docId, info.size);
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
    const ipcErr = err instanceof IpcError ? err : null;
    const code = ipcErr?.payload.code ?? "";
    if (code !== "conflict" && code !== "unmappable") {
      reportError(err);
      return;
    }
    // Cross-platform async dialog (I4): window.confirm is unreliable on
    // WKWebView / webkit2gtk.
    setBusy(false);
    if (code === "conflict") {
      // Three-way (spec §4.4): reload discards local edits, overwrite forces
      // the write, cancel keeps editing. `ask` only has two buttons.
      const choice = await confirmExternalConflict(ipcErr!.zh);
      if (choice === "reload") {
        await reloadDocFromDisk(a.info.docId, ds.path);
        return;
      }
      if (choice === "cancel") return;
    } else {
      // Encoding gate keeps the two-button informed prompt (spec §5.4).
      const go = await ask(`${ipcErr!.zh}\n\n确定要强制覆盖吗？`, {
        title: "编码无法表示",
        kind: "warning",
        okLabel: "强制覆盖",
        cancelLabel: "取消",
      });
      if (!go) return;
    }
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
    return;
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
  // A successful write resolves any external-change warning.
  tabs.setExternalFlag(docId, false);
  hideExtBar(docId);
  // Save-As may have moved the file to a directory that failed to watch.
  void refreshWatchStatus();
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

function applyMode(docId: number, mode: string): void {
  const ds = docStates.get(docId);
  const handle = editors.get(docId); // read-only docs have no syntax mode
  if (!ds || !handle) return;
  // `mode` comes from the dropdown / caller as a plain string (builtin name or
  // `plugin:<id>`); only AUTO needs resolution against the doc's extension.
  const resolved = (mode === "auto" ? autoModeId(extOf(ds.path)) : mode) as ModeId | null;
  handle.setMode(resolved === "plain" ? null : resolved); // reconfigure language only
}

async function handleClose(docId: number): Promise<boolean> {
  const view = tabs.get(docId);
  if (!view) return false;
  if (view.isDirty()) {
    const go = await ask("文档有未保存改动，确定关闭？", {
      title: "关闭文档",
      kind: "warning",
      okLabel: "丢弃并关闭",
      cancelLabel: "取消",
    });
    if (!go) return false;
  }
  const closing = docStates.get(docId);
  if (closing) cancelRecovery(docId, closing.path);
  if (parserReqId !== null) stopParserRun(); // results are per-run, not per-doc
  hideExtBar(docId);
  followStates.delete(docId);
  tailToast.delete(docId);
  tailSeenSize.delete(docId);
  if (followResumeOwner === docId) hideFollowResume();
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
  saveSession();
  return true;
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
  // Follow UI belongs to the ACTIVE doc only (syncStatus also runs for
  // background reloads); a hidden tab may be followed but its chip stays hidden.
  if (docId !== tabs.active()?.info.docId) return;
  if (handle) {
    status.setFollowAvailable(false);
    hideFollowResume();
    return;
  }
  const st = followStates.get(docId);
  status.setFollowAvailable(true);
  status.setFollow(st?.on ?? false);
  if (st?.on && st.paused) {
    showFollowResume(docId);
  } else {
    hideFollowResume();
    if (st?.on) lists.get(docId)?.scrollToBottom();
  }
}

// ---- tail follow (Roadmap: 监控 tail 与滚动截断) ---------------------------

function setFollow(on: boolean): void {
  const a = tabs.active();
  if (!a) return;
  const docId = a.info.docId;
  const st = followStates.get(docId) ?? { on: false, paused: false };
  st.on = on;
  st.paused = false;
  followStates.set(docId, st);
  status.setFollow(on);
  if (on) lists.get(docId)?.scrollToBottom();
  else hideFollowResume();
}

function onListScroll(docId: number, top: number, maxScroll: number): void {
  const st = followStates.get(docId);
  if (!st || !st.on) return;
  if (docId !== tabs.active()?.info.docId) return;
  if (isNearBottom(top, maxScroll)) {
    if (st.paused) {
      st.paused = false;
      hideFollowResume();
    }
  } else if (!st.paused) {
    st.paused = true;
    showFollowResume(docId);
  }
}

function showFollowResume(docId: number): void {
  if (!followResumeEl) {
    followResumeEl = mkBtn("回到底部 ↓", "恢复跟随最新行", () => {
      const owner = followResumeOwner;
      if (owner === null) return;
      const st = followStates.get(owner);
      if (st) st.paused = false;
      lists.get(owner)?.scrollToBottom();
      hideFollowResume();
    });
    followResumeEl.className = "follow-resume";
    document.body.append(followResumeEl);
  }
  followResumeOwner = docId;
  followResumeEl.style.display = "";
}

function hideFollowResume(): void {
  followResumeOwner = null;
  if (followResumeEl) followResumeEl.style.display = "none";
}

/** Read-only external change: extend in place, falling back to a full reload. */
async function handleReadonlyAppend(docId: number, path: string): Promise<void> {
  try {
    const r = await docTailAppend(docId);
    const ds = docStates.get(docId);
    const list = lists.get(docId);
    if (!ds || !list) return; // closed while polling
    const prevSize = tailSeenSize.get(docId);
    tailSeenSize.set(docId, r.size);
    if (r.rotated) {
      await reloadDocFromDisk(docId, path);
      if (followStates.get(docId)?.on) lists.get(docId)?.scrollToBottom();
      return;
    }
    if (r.linesAdded === 0) {
      // A half-written tail (no LF yet): no new line, but the last line grew.
      if (prevSize !== undefined && r.size > prevSize) {
        list.invalidate();
        list.refresh();
        const st = followStates.get(docId);
        if (st?.on && !st.paused) list.scrollToBottom();
      }
      return;
    }
    ds.totalLines = r.totalLines;
    indexed.set(docId, r.totalLines);
    list.refresh();
    const st = followStates.get(docId);
    if (st?.on && !st.paused) list.scrollToBottom();
    if (docId === tabs.active()?.info.docId) {
      status.setReadonly(r.totalLines);
      updateInspector(docId);
    }
    const t = tailToast.get(docId) ?? { pending: 0, at: 0 };
    t.pending += r.linesAdded;
    const now = Date.now();
    if (now - t.at >= 1000) {
      toastMsg(`已追加 ${t.pending} 行：${basenameOf(path)}`);
      t.at = now;
      t.pending = 0;
    }
    tailToast.set(docId, t);
  } catch (err) {
    reportError(err);
    const st = followStates.get(docId);
    if (st) {
      st.on = false;
      st.paused = false;
    }
    if (docId === tabs.active()?.info.docId) status.setFollow(false);
    if (followResumeOwner === docId) hideFollowResume();
  }
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
    if (e.key === "Enter" && !isIMEEvent(e)) onEnter();
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
    if (e.key === "Enter" && !isIMEEvent(e)) submit();
    else if (e.key === "Escape") close();
  });
  panel.append(input, go);
  document.getElementById("app")?.append(panel);
  input.focus();
  input.select();
}

/** Single action source: palette rows, the keymap modal and dispatch all use it. */
type ActionDef = { id: ActionId; title: string; run: () => void };
const ACTION_DEFS: ActionDef[] = [
  { id: "open", title: "打开文件…", run: () => void openFile() },
  { id: "save", title: "保存", run: () => void saveActive(false) },
  { id: "save-as", title: "另存为…", run: () => void saveActive(true) },
  { id: "query", title: "行过滤 / 高亮命中…", run: () => openQueryPanel() },
  { id: "jump", title: "跳转到行…", run: () => openJumpPrompt() },
  { id: "palette", title: "命令面板", run: () => openPalette() },
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
];

function actionTitle(id: string): string {
  return ACTION_DEFS.find((d) => d.id === id)?.title ?? id;
}

/** Load `[keymap]` overrides once at startup and surface conflicts/unknown ids. */
async function initKeymap(): Promise<void> {
  try {
    keymapOverrides = await keymapGet();
  } catch {
    keymapOverrides = {};
    toastMsg("快捷键配置读取失败，已使用默认值");
    keymapState = mergeBindings({});
    return;
  }
  applyKeymapState();
}

/** Recompute the dispatch tables from the current overrides (default defaults). */
function applyKeymapState(): void {
  keymapState = mergeBindings(keymapOverrides);
  const notes: string[] = [];
  const c = keymapState.conflicts[0];
  if (c) {
    notes.push(
      `快捷键冲突：${c.accel} 已绑给《${actionTitle(c.other)}》，《${actionTitle(c.action)}》未生效`,
    );
  }
  if (keymapState.unknown.length) {
    notes.push(`配置文件含未知动作：${keymapState.unknown.join("、")}`);
  }
  if (keymapState.invalid.length) {
    const hasDefault = keymapState.invalid.filter((id) => DEFAULT_BINDINGS[id as ActionId]);
    const unbound = keymapState.invalid.filter((id) => !DEFAULT_BINDINGS[id as ActionId]);
    const parts: string[] = [];
    if (hasDefault.length) parts.push(`已用默认：${hasDefault.map(actionTitle).join("、")}`);
    if (unbound.length) parts.push(`已置未绑定：${unbound.map(actionTitle).join("、")}`);
    notes.push(`配置文件中的快捷键无效（${parts.join("；")}）`);
  }
  if (notes.length) toastMsg(notes.join("；"));
}

/** Persist one binding and return the freshly merged table (modal redraws). */
async function applyKeymap(
  action: ActionId,
  accel: string | null,
): Promise<Partial<Record<ActionId, string>>> {
  try {
    await keymapSet(action, accel);
  } catch (err) {
    reportError(err);
    throw err; // the modal keeps showing the old value
  }
  if (accel === null) delete keymapOverrides[action];
  else keymapOverrides[action] = accel;
  applyKeymapState();
  return keymapState.bindings;
}

/** Drop every override (back to defaults). */
async function resetKeymapAll(): Promise<Partial<Record<ActionId, string>>> {
  try {
    await keymapReset();
  } catch (err) {
    reportError(err);
    throw err;
  }
  keymapOverrides = {};
  applyKeymapState();
  return keymapState.bindings;
}

function openKeymapSettings(): void {
  if (document.querySelector(".keymap-modal") || document.querySelector(".script-modal")) return;
  keymapModalOpen = true;
  openKeymapModal({
    actions: ACTION_DEFS.map((d) => ({ id: d.id, title: d.title })),
    bindings: keymapState.bindings,
    onSet: applyKeymap,
    onResetAll: resetKeymapAll,
    macros: savedMacros.map((m) => ({ name: m.name, accel: macroBindings[m.name] ?? "" })),
    onMacroSet: async (name, accel) => {
      if (accel === null || accel === "") delete macroBindings[name];
      else macroBindings[name] = accel;
      persistMacroBindings();
      return { ...macroBindings };
    },
    onClose: () => {
      keymapModalOpen = false;
    },
  });
}

/** Script manager (Roadmap-3): all IO/dialog work is injected into the modal. */
function openScriptManager(): void {
  if (scriptModalOpen || document.querySelector(".script-modal") || document.querySelector(".keymap-modal")) {
    return;
  }
  scriptModalOpen = true;
  try {
    openScriptModal({
    hasDocument: () => tabs.active() !== null,
    defaultOutputBase: () => {
      const a = tabs.active();
      const ds = a ? docStates.get(a.info.docId) : undefined;
      return ds ? basenameOf(ds.path) : "script";
    },
    list: scriptList,
    read: scriptRead,
    write: scriptWrite,
    remove: scriptDelete,
    chooseTarget: async (base) => {
      const picked = await save({
        defaultPath: `${base}.out.txt`,
        filters: [{ name: "文本", extensions: ["txt"] }],
      });
      return typeof picked === "string" ? picked : null;
    },
    run: (a) => {
      const active = tabs.active();
      if (!active) return Promise.reject(new Error("请先打开一个文档"));
      return scriptRun({
        docId: active.info.docId,
        code: a.code,
        targetPath: a.targetPath,
        onConsole: a.onConsole,
        onProgress: a.onProgress,
        onDone: a.onDone,
      });
    },
    cancel: scriptCancel,
    confirm: (msg) => ask(msg, { title: "脚本", kind: "warning" }),
    onClose: () => {
      scriptModalOpen = false;
    },
    });
  } catch (err) {
    scriptModalOpen = false; // never leave shortcuts disabled on a failed open
    reportError(err);
  }
}

/** The command set offered by the palette, rebuilt per open so it reflects
 *  live state (e.g. hints). Titles are what `fuzzyScore` matches. */
function buildCommands(): Command[] {
  const bound: Command[] = ACTION_DEFS.map((d) => ({
    id: d.id,
    title: d.title,
    hint: keymapState.bindings[d.id],
    run: d.run,
  }));
  // Recent files at the top: filename as the searchable title, full path as
  // the hint. Ids use a `recent-` prefix — outside the keymap action space.
  const recents: Command[] = recentPaths
    .slice(0, 10)
    .map((p, i) => ({
      id: `recent-${i}`,
      title: `最近：${basenameOf(p)}`,
      hint: p,
      run: () => void openPath(p),
    }));
  return [
    ...recents,
    ...bound,
    { id: "recent-clear", title: "清除最近文件记录", run: () => void clearRecentList() },
    { id: "keymap", title: "快捷键设置…", run: () => openKeymapSettings() },
    { id: "script", title: "脚本…", run: () => openScriptManager() },
    { id: "reload-plugins", title: "重载语法插件", run: () => void initSyntaxPlugins() },
    { id: "wrap", title: "切换自动换行", run: toggleEditorWrap },
    { id: "font-in", title: "放大编辑器字体", run: () => activeEditor()?.zoomFont(1) },
    { id: "font-out", title: "缩小编辑器字体", run: () => activeEditor()?.zoomFont(-1) },
    { id: "font-reset", title: "重置编辑器字体", run: () => activeEditor()?.fontReset() },
    { id: "sort-asc", title: "行排序（升序）", run: () => activeEditor()?.lineOp("sort-asc") },
    { id: "sort-desc", title: "行排序（降序）", run: () => activeEditor()?.lineOp("sort-desc") },
    { id: "dedupe", title: "行去重", run: () => activeEditor()?.lineOp("dedupe") },
    { id: "reverse-lines", title: "行反转", run: () => activeEditor()?.lineOp("reverse") },
    { id: "upper", title: "转大写", run: () => activeEditor()?.lineOp("upper") },
    { id: "lower", title: "转小写", run: () => activeEditor()?.lineOp("lower") },
    { id: "trim", title: "Trim 行首尾空格", run: () => activeEditor()?.lineOp("trim") },
    { id: "drop-empty", title: "删除空行", run: () => activeEditor()?.lineOp("drop-empty") },
    { id: "join-lines", title: "合并为一行", run: () => activeEditor()?.lineOp("join") },
    { id: "bookmark-toggle", title: "书签：切换当前行", run: () => activeEditor()?.toggleBookmark() },
    { id: "bookmark-next", title: "书签：下一条", run: () => activeEditor()?.nextBookmark() },
    { id: "bookmark-prev", title: "书签：上一条", run: () => activeEditor()?.prevBookmark() },
    { id: "bookmark-clear", title: "书签：清除全部", run: () => activeEditor()?.clearBookmarks() },
    {
      id: "macro-start",
      title: "宏：开始录制",
      run: () => {
        const h = activeEditor();
        if (!h) return;
        toastMsg(h.startRecording() ? "开始录制宏" : "已在录制中");
      },
    },
    {
      id: "macro-stop",
      title: "宏：停止录制",
      run: () => {
        const h = activeEditor();
        if (!h) return;
        const n = h.stopRecording();
        toastMsg(`已停止录制（${n} 个操作）`);
      },
    },
    { id: "macro-play", title: "宏：播放", run: () => activeEditor()?.playMacro(1) },
    // Plan 41: each saved macro as a palette command, hint = bound key (if any).
    ...savedMacros.map((m) => ({
      id: `macro-cmd-${m.name}`,
      title: `宏：${m.name}`,
      hint: macroBindings[m.name],
      run: () => activeEditor()?.playMacro(1, m.ops),
    })),
    {
      id: "macro-play-times",
      title: "宏：重复播放 N 次…",
      run: () => {
        const h = activeEditor();
        if (!h) return;
        openCommandList("输入重复次数后回车（1–10000）", [], "macro-play-times-list", {
          onFreeText: (raw) => {
            const n = parseRepeatCount(raw);
            if (n === null) {
              toastMsg("次数需为 1–10000 的整数");
              return;
            }
            h.playMacro(n);
          },
        });
      },
    },
    {
      id: "macro-run-to-end",
      title: "宏：运行到文件尾（无进展即停）",
      run: () => {
        const h = activeEditor();
        if (!h) return;
        const runs = h.playMacroToEnd();
        toastMsg(runs === 0 ? "没有已录制的宏" : `已连续播放 ${runs} 次`);
      },
    },
    {
      id: "macro-save",
      title: "宏：保存为…",
      run: () => {
        const h = activeEditor();
        const ops = h?.macroOps() ?? [];
        if (ops.length === 0) {
          toastMsg("没有已录制的宏");
          return;
        }
        openCommandList("输入宏名称后回车保存", [], "macro-save-list", {
          onFreeText: (raw) => {
            const name = raw.trim();
            if (!name) return;
            const i = savedMacros.findIndex((m) => m.name === name);
            if (i >= 0) savedMacros[i] = { name, ops };
            else savedMacros.push({ name, ops });
            if (savedMacros.length > MACRO_STORE_MAX) {
              savedMacros = savedMacros.slice(-MACRO_STORE_MAX);
            }
            persistMacros();
            toastMsg(`已保存宏「${name}」（${ops.length} 个操作）`);
          },
        });
      },
    },
    {
      id: "macro-run",
      title: "宏：运行已保存宏",
      run: () => {
        if (savedMacros.length === 0) {
          toastMsg("还没有保存的宏");
          return;
        }
        openCommandList(
          "选择要运行的宏…",
          savedMacros.map((m) => ({
            id: `macro-run-${m.name}`,
            title: m.name,
            hint: `${m.ops.length} 个操作`,
            run: () => activeEditor()?.playMacro(1, m.ops),
          })),
          "macro-run-list",
        );
      },
    },
    {
      id: "macro-delete",
      title: "宏：删除已保存宏",
      run: () => {
        if (savedMacros.length === 0) {
          toastMsg("还没有保存的宏");
          return;
        }
        openCommandList(
          "选择要删除的宏…",
          savedMacros.map((m) => ({
            id: `macro-del-${m.name}`,
            title: m.name,
            run: () => {
              savedMacros = savedMacros.filter((x) => x.name !== m.name);
              if (macroBindings[m.name]) {
                delete macroBindings[m.name];
                persistMacroBindings();
              }
              persistMacros();
              toastMsg(`已删除宏「${m.name}」`);
            },
          })),
          "macro-del-list",
        );
      },
    },
    {
      id: "minimap",
      title: "切换概览条（文档地图）",
      run: () => {
        const h = activeEditor();
        if (h) toastMsg(h.setMinimap(!h.isMinimap()) ? "概览条：开" : "概览条：关");
      },
    },
    {
      id: "symbol-list",
      title: "函数列表…",
      run: () => {
        const h = activeEditor();
        if (!h) return;
        const symbols = h.listSymbols();
        if (symbols.length === 0) {
          toastMsg("当前语法没有符号规则");
          return;
        }
        openCommandList(
          "过滤符号…",
          symbols.map((s) => ({
            id: `sym-${s.line}-${s.name}`,
            title: s.name,
            hint: `行 ${s.line} · ${s.kind}`,
            run: () => h.revealLine(s.line),
          })),
          "symbols",
        );
      },
    },
    { id: "mf4", title: "测量数据…", run: () => void openMf4File() },
    { id: "theme-light", title: "主题：浅色", run: () => theme.set("light") },
    { id: "theme-dark", title: "主题：暗黑", run: () => theme.set("dark") },
    { id: "theme-vampire", title: "主题：吸血鬼 (Dracula)", run: () => theme.set("vampire") },
    { id: "theme-auto", title: "主题：跟随系统", run: () => theme.set("auto") },
  ];
}

/** Ctrl+Shift+P palette: a fuzzy-filtered, keyboard-navigable command list. */
function openPalette(): void {
  openCommandList("输入命令…", buildCommands(), "palette");
}


/**
 * Generic fuzzy-filtered list overlay (palette + function list share it).
 * `domId` lets an already-open instance just reclaim focus. With `onFreeText`,
 * Enter on an empty (unmatched) list submits the raw input text instead —
 * macro naming rides the same panel (Plan 30).
 */
function openCommandList(
  placeholder: string,
  cmds: Command[],
  domId: string,
  opts?: { onFreeText?: (text: string) => void },
): void {
  const already = document.getElementById(domId);
  if (already) {
    (already.querySelector("input") as HTMLInputElement | null)?.focus();
    return;
  }
  const overlay = document.createElement("div");
  overlay.id = domId;
  overlay.className = "palette-overlay";
  const box = document.createElement("div");
  box.className = "palette-box";
  const input = document.createElement("input");
  input.type = "text";
  input.className = "palette-input";
  input.placeholder = placeholder;
  const list = document.createElement("div");
  list.className = "palette-list";
  box.append(input, list);
  overlay.append(box);
  document.getElementById("app")?.append(overlay);

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
    } else if (e.key === "Enter" && !isIMEEvent(e)) {
      e.preventDefault();
      if (view.length) {
        run(sel);
      } else if (opts?.onFreeText && input.value.trim() !== "") {
        const text = input.value;
        close();
        opts.onFreeText(text);
      }
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
    if (keymapModalOpen || scriptModalOpen) return; // a modal owns the keyboard
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
    const accel = eventToAccel(e);
    if (!accel) return;
    const action = keymapState.accelToAction[accel];
    if (!action) {
      // Plan 41: a macro binding is the last dispatcher after the fixed actions.
      const macroName = macroForAccel(accel, macroBindings);
      const macro = macroName ? savedMacros.find((m) => m.name === macroName) : undefined;
      if (macro) {
        e.preventDefault();
        activeEditor()?.playMacro(1, macro.ops);
      }
      return;
    }
    const def = ACTION_DEFS.find((d) => d.id === action);
    if (!def) return;
    e.preventDefault();
    def.run();
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
void initWatch();
void initSyntaxPlugins();
void initKeymap();
void initRecent();
void restoreSession();
initDragDrop();
