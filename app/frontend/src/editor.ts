//! Edit-mode editor (spec §4.3 / §8, ≤200MB path). `openEditable` pulls the
//! whole document through `doc_chunk` and mounts a CodeMirror 6 `EditorView`.
//! Exposes a small handle the tab/status-bar layer drives; no IPC knowledge
//! lives here beyond `loadAllText`. EOL is a save-time concern carried on the
//! handle (the shell owns line-ending write-back), seeded from the core's
//! detected style so a CRLF file is not silently rewritten to LF (spec §4.4).

import { EditorState, Compartment, StateEffect, StateField, type Transaction } from "@codemirror/state";
import {
  Decoration,
  EditorView,
  gutter,
  keymap,
  lineNumbers,
  highlightActiveLine,
  drawSelection,
  rectangularSelection,
  crosshairCursor,
  GutterMarker,
  type DecorationSet,
} from "@codemirror/view";
import {
  cursorCharLeft,
  cursorCharRight,
  cursorLineDown,
  cursorLineUp,
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,
} from "@codemirror/commands";
import {
  searchKeymap,
  search,
  openSearchPanel,
  highlightSelectionMatches,
} from "@codemirror/search";
import {
  bracketMatching,
  foldGutter,
  indentOnInput,
  syntaxHighlighting,
  defaultHighlightStyle,
  type LanguageSupport,
} from "@codemirror/language";
import { loadAllText, type DocInfo } from "./ipc";
import { autoModeId, resolveLanguage, type ModeId } from "./language";
import {
  applyLineOp,
  bookmarkNext,
  bookmarkPrev,
  bookmarkToggle,
  clampFontSize,
  FONT_BASE,
  type LineOp,
} from "./editorops";
import {
  classifyMove,
  extractMacroOps,
  extractSymbols,
  lineAtY,
  overviewBuckets,
  MACRO_MAX_OPS,
  MACRO_PLAY_MAX,
  OVERVIEW_MAX_LINES,
  type MacroOp,
  type MoveBy,
  type SymbolItem,
} from "./editortools";

/** Editor font-size theme fragment (Plan 27 zoom); rebuilt on every step. */
function fontTheme(px: number) {
  return EditorView.theme({ "&": { fontSize: `${px}px` } });
}

// ---- bookmarks (Plan 28) ----------------------------------------------------

/** Sorted char-offset anchors; follows edits via change mapping. */
const setBookmarks = StateEffect.define<readonly number[]>();
const bookmarksPos = StateField.define<readonly number[]>({
  create: () => [],
  update(value, tr) {
    let v = value.map((p) => tr.changes.mapPos(p));
    for (const e of tr.effects) if (e.is(setBookmarks)) v = [...e.value];
    return v;
  },
});
const bookmarkDeco = EditorView.decorations.compute([bookmarksPos], (state) => {
  const bms = state.field(bookmarksPos, false) ?? [];
  const deco: DecorationSet = Decoration.set(
    bms.map((pos) => Decoration.line({ class: "cm-bookmarked-line" }).range(pos)),
    true,
  );
  return deco;
});
class BookmarkMarker extends GutterMarker {
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-bookmark-marker";
    span.textContent = "●";
    return span;
  }
}
const bookmarkMarker = new BookmarkMarker();
const bookmarkGutter = gutter({
  class: "cm-bookmark-gutter",
  lineMarker: (view, line, _other) =>
    (view.state.field(bookmarksPos, false) ?? []).includes(line.from) ? bookmarkMarker : null,
});

function toggleBookmarkCmd(view: EditorView): boolean {
  const pos = view.state.doc.lineAt(view.state.selection.main.head).from;
  view.dispatch({ effects: setBookmarks.of(bookmarkToggle(view.state.field(bookmarksPos), pos)) });
  return true;
}

function jumpBookmark(view: EditorView, pos: number): boolean {
  view.dispatch({
    selection: { anchor: pos },
    effects: EditorView.scrollIntoView(pos, { y: "center" }),
  });
  view.focus();
  return true;
}

function nextBookmarkCmd(view: EditorView): boolean {
  const b = bookmarkNext(view.state.field(bookmarksPos), view.state.selection.main.head);
  return b === null ? false : jumpBookmark(view, b);
}

function prevBookmarkCmd(view: EditorView): boolean {
  const b = bookmarkPrev(view.state.field(bookmarksPos), view.state.selection.main.head);
  return b === null ? false : jumpBookmark(view, b);
}

function clearBookmarksCmd(view: EditorView): boolean {
  view.dispatch({ effects: setBookmarks.of([]) });
  return true;
}

/** Read-only bookmark anchors (sorted char offsets) — test/inspection hook. */
export function bookmarkPositions(view: EditorView): readonly number[] {
  return view.state.field(bookmarksPos, false) ?? [];
}

export interface EditorHandle {
  readonly docId: number;
  readonly view: EditorView;
  /** Same element as `view.dom` — lets `TabStrip` treat editors/lists uniformly. */
  readonly root: HTMLElement;
  getText(): string;
  isDirty(): boolean;
  markSaved(): void;
  /** Replace the whole buffer (encoding re-decode re-render); marks dirty. */
  replaceText(text: string): void;
  /** Reconfigure only the language extension — keeps undo history + scroll. */
  setMode(mode: ModeId | null): void;
  setEol(eol: string): void;
  eol(): string;
  focus(): void;
  openSearch(): void;
  /** Scroll a 1-based source line into view and place the cursor there (CanPanel §7.4). */
  revealLine(line1: number): void;
  /** Toggle word wrap (Plan 27). */
  setWrap(on: boolean): void;
  /** Current wrap flag (kept in JS; jsdom cannot observe the CSS effect). */
  isWrap(): boolean;
  /** One line operation over the selection's line range (whole doc if empty). */
  lineOp(op: LineOp): void;
  /** Nudge the editor font size (Plan 27 zoom); clamped. */
  zoomFont(delta: number): void;
  /** Back to the base font size. */
  fontReset(): void;
  /** Bookmark the cursor line (Plan 28); no-op if already bookmarked. */
  toggleBookmark(): void;
  /** Jump to the next/previous bookmark (wraparound); false when none. */
  nextBookmark(): boolean;
  prevBookmark(): boolean;
  clearBookmarks(): void;
  // ---- Plan 29 editor tools ----
  /** Begin capturing user input transactions; false if already recording. */
  startRecording(): boolean;
  /** Stop and return the captured op count. */
  stopRecording(): number;
  isRecording(): boolean;
  /** Copy of the last finished macro (empty while recording). */
  macroOps(): MacroOp[];
  /**
   * Replay a macro `times` times from the current cursor — the last recorded
   * one, or `ops` (a saved macro) when given (Plan 30).
   */
  playMacro(times?: number, ops?: MacroOp[]): void;
  /**
   * Replay the last recorded macro until neither doc length nor the cursor
   * moves any more; returns the number of plays (Plan 31). Hard-capped.
   */
  playMacroToEnd(): number;
  /** Toggle the overview strip; returns the new state. */
  setMinimap(on: boolean): boolean;
  isMinimap(): boolean;
  /** Symbols of the CURRENT mode (empty when the mode has no rules). */
  listSymbols(): SymbolItem[];
  destroy(): void;
}

export interface OpenEditableOpts {
  parent: HTMLElement;
  info: DocInfo;
  /** File extension (no dot) used to pick the starting syntax mode. */
  ext: string;
  /** Initial word-wrap state (persisted UI flag; default off = CodeMirror default). */
  wrap?: boolean;
  /** Fires once on the first user edit (drives the tab dirty dot). */
  onDirty?: () => void;
  /** Fires on EVERY edit (drives the 2s crash-recovery idle timer). */
  onEdit?: () => void;
  /** Decoded U+FFFD replacement count from the full load (spec §5). */
  onInvalid?: (count: number) => void;
}

/**
 * Load and mount `info` for editing. Fetches every block (async) before building
 * the view, so the first paint shows the full document. Returns a handle once
 * the editor is live.
 */
export async function openEditable(opts: OpenEditableOpts): Promise<EditorHandle> {
  const total = opts.info.totalLines ?? opts.info.lineCountEstimate;
  const { text, invalidCount } = await loadAllText(opts.info.docId, total);
  opts.onInvalid?.(invalidCount);

  let dirty = false;
  let eol = opts.info.eol || "lf";
  // Language lives in a Compartment so a mode switch reconfigures it in place —
  // `setState` would drop undo history, scroll, and wrongly mark the doc dirty.
  const langConf = new Compartment();
  // Wrap + font-size live in Compartments too (Plan 27): both toggle at runtime
  // without rebuilding the view (undo/scroll survive).
  const wrapConf = new Compartment();
  const fontConf = new Compartment();
  let wrapOn = opts.wrap ?? false;
  // Plan 29 editor tools state.
  let macro: MacroOp[] | null = null; // non-null while recording
  let recorded: MacroOp[] = []; // last finished macro
  let minimapOn = true;
  let currentMode: ModeId | null = null;
  let overviewRedrawQueued = false;
  // Language packages load on demand, so a mode switch resolves asynchronously;
  // the token makes a slow early request lose to a later one instead of
  // overwriting it.
  let modeToken = 0;
  let destroyed = false;

  const onDocChanged = () => {
    const first = !dirty;
    dirty = true;
    opts.onEdit?.();
    if (first) opts.onDirty?.();
  };

  const lang = await resolveLanguage(autoModeId(opts.ext));
  // Single font-size holder shared by the wheel handler and `zoomFont`.
  const zoom = { size: FONT_BASE };
  const view = new EditorView({
    state: buildState(
      text,
      lang,
      langConf,
      wrapConf,
      fontConf,
      zoom,
      opts.wrap ?? false,
      onDocChanged,
      {
        capture: (tr) => {
          if (macro === null || macro.length >= MACRO_MAX_OPS) return;
          // Backspace must replay as a backward delete, not a forward one.
          const dir = tr.isUserEvent("delete.backward") ? "backward" : "forward";
          tr.changes.iterChangedRanges((fromA, toA, fromB, toB) => {
            if (macro!.length >= MACRO_MAX_OPS) return;
            macro!.push(
              ...extractMacroOps(
                [{ removed: toA - fromA, inserted: tr.state.sliceDoc(fromB, toB) }],
                dir,
              ),
            );
          });
        },
        captureMove: (tr) => {
          if (macro === null || macro.length >= MACRO_MAX_OPS) return;
          const before = tr.startState.selection.main.head;
          const after = tr.state.selection.main.head;
          if (before === after) return;
          const bl = tr.startState.doc.lineAt(before);
          const al = tr.state.doc.lineAt(after);
          const by = classifyMove(
            { line: bl.number, col: before - bl.from },
            { line: al.number, col: after - al.from },
            al.length,
          );
          if (by) macro.push({ kind: "move", by });
        },
        queueOverview: queueOverviewRedraw,
        minimapOn: () => minimapOn,
      },
    ),
    parent: opts.parent,
  });

  // ---- overview strip (Plan 29) ----------------------------------------------

  const overview = document.createElement("canvas");
  overview.className = "cm-overview";
  view.dom.appendChild(overview);
  const viewDom = view.dom as HTMLElement & { style: CSSStyleDeclaration };
  viewDom.style.position = "relative";

  function queueOverviewRedraw(): void {
    if (overviewRedrawQueued || destroyed) return;
    overviewRedrawQueued = true;
    requestAnimationFrame(() => {
      overviewRedrawQueued = false;
      drawOverview();
    });
  }

  function drawOverview(): void {
    const px = overview.clientHeight;
    const dpr = window.devicePixelRatio || 1;
    overview.width = Math.max(1, Math.trunc(overview.clientWidth * dpr));
    overview.height = Math.max(1, Math.trunc(px * dpr));
    const ctx = overview.getContext("2d");
    if (!ctx || px <= 0) return;
    const d = view.state.doc;
    const total = d.lines;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, overview.clientWidth, px);
    if (total > OVERVIEW_MAX_LINES) return; // guard: viewport frame only
    const buckets = overviewBuckets(total, px);
    const w = overview.clientWidth;
    for (const [from, to] of buckets) {
      // Non-empty line ratio → intensity; blank runs show as background.
      let nonEmpty = 0;
      for (let n = from; n <= to; n++) if (d.line(n).text.trim() !== "") nonEmpty++;
      const ratio = nonEmpty / Math.max(1, to - from + 1);
      const yTop = ((from - 1) / total) * px;
      const yH = Math.max(1, ((to - from + 1) / total) * px);
      ctx.fillStyle = `rgba(128,140,155,${0.25 + ratio * 0.55})`;
      ctx.fillRect(0, yTop, w, yH);
    }
    // Viewport window.
    const ranges = view.visibleRanges;
    if (ranges.length > 0) {
      const yTop = ((d.lineAt(ranges[0]!.from).number - 1) / total) * px;
      const yBot = (d.lineAt(ranges[ranges.length - 1]!.to).number / total) * px;
      ctx.fillStyle = "rgba(47,111,235,0.22)";
      ctx.fillRect(0, yTop, w, Math.max(2, yBot - yTop));
    }
  }

  function overviewJump(clientY: number): void {
    const rect = overview.getBoundingClientRect();
    const line = lineAtY(clientY - rect.top, rect.height, view.state.doc.lines);
    const ln = view.state.doc.line(Math.max(1, Math.min(line, view.state.doc.lines)));
    view.dispatch({
      selection: { anchor: ln.from },
      effects: EditorView.scrollIntoView(ln.from, { y: "center" }),
    });
  }
  overview.addEventListener("pointerdown", (e) => {
    overview.setPointerCapture(e.pointerId);
    overviewJump(e.clientY);
    const move = (ev: PointerEvent) => overviewJump(ev.clientY);
    const up = () => {
      overview.removeEventListener("pointermove", move);
      overview.removeEventListener("pointerup", up);
    };
    overview.addEventListener("pointermove", move);
    overview.addEventListener("pointerup", up);
  });

  return {
    docId: opts.info.docId,
    view,
    root: view.dom,
    getText: () => view.state.doc.toString(),
    isDirty: () => dirty,
    markSaved: () => {
      dirty = false;
    },
    replaceText: (next: string) => {
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: next },
      });
      dirty = true;
      opts.onEdit?.();
      opts.onDirty?.();
    },
    setMode: (mode: ModeId | null) => {
      currentMode = mode;
      const token = ++modeToken;
      void resolveLanguage(mode).then((next) => {
        if (destroyed || token !== modeToken) return;
        view.dispatch({ effects: langConf.reconfigure(next ?? []) });
      });
    },
    setEol: (next: string) => {
      eol = next;
      dirty = true;
      opts.onEdit?.();
      opts.onDirty?.();
    },
    eol: () => eol,
    focus: () => view.focus(),
    openSearch: () => openSearchPanel(view),
    revealLine: (line1: number) => {
      // Clamp to a real line, then jump the cursor + center it — the 1-based
      // contract matches FrameJson.lineNumber (CodeMirror lines are 1-based).
      const ln = view.state.doc.line(Math.max(1, Math.min(line1, view.state.doc.lines)));
      view.dispatch({
        selection: { anchor: ln.from },
        effects: EditorView.scrollIntoView(ln.from, { y: "center" }),
      });
      view.focus();
    },
    setWrap: (on: boolean) => {
      wrapOn = on;
      view.dispatch({ effects: wrapConf.reconfigure(on ? EditorView.lineWrapping : []) });
    },
    isWrap: () => wrapOn,
    lineOp: (op: LineOp) => {
      if (macro !== null && macro.length < MACRO_MAX_OPS) macro.push({ kind: "line", op });
      applyLineOpToView(view, op);
    },
    zoomFont: (delta: number) => {
      zoom.size = clampFontSize(zoom.size + delta);
      view.dispatch({ effects: fontConf.reconfigure(fontTheme(zoom.size)) });
    },
    fontReset: () => {
      zoom.size = FONT_BASE;
      view.dispatch({ effects: fontConf.reconfigure(fontTheme(zoom.size)) });
    },
    toggleBookmark: () => void toggleBookmarkCmd(view),
    nextBookmark: () => nextBookmarkCmd(view),
    prevBookmark: () => prevBookmarkCmd(view),
    clearBookmarks: () => void clearBookmarksCmd(view),
    startRecording: () => {
      if (macro !== null) return false;
      macro = [];
      return true;
    },
    stopRecording: () => {
      const ops = macro ?? [];
      recorded = ops;
      macro = null;
      return ops.length;
    },
    isRecording: () => macro !== null,
    macroOps: () => recorded.slice(),
    playMacro: (times = 1, ops?: MacroOp[]) => {
      replayOps(view, ops ?? recorded, times);
    },
    setMinimap: (on: boolean) => {
      minimapOn = on;
      overview.style.display = on ? "" : "none";
      if (on) queueOverviewRedraw();
      return minimapOn;
    },
    isMinimap: () => minimapOn,
    playMacroToEnd: () => {
      if (recorded.length === 0) return 0;
      let runs = 0;
      for (; runs < MACRO_PLAY_MAX; runs++) {
        const before = {
          len: view.state.doc.length,
          head: view.state.selection.main.head,
        };
        // reuse the same playback path (ops default to `recorded`)
        replayOps(view, recorded, 1);
        const after = {
          len: view.state.doc.length,
          head: view.state.selection.main.head,
        };
        if (after.len === before.len && after.head === before.head) {
          runs++; // that last play did happen, it just made no progress
          break;
        }
      }
      return runs;
    },
    listSymbols: () =>
      extractSymbols(
        Array.from({ length: view.state.doc.lines }, (_, i) => {
          const line = view.state.doc.line(i + 1);
          return { n: i + 1, text: line.text };
        }),
        typeof currentMode === "string" && !currentMode.startsWith("plugin:")
          ? currentMode
          : null,
      ),
    destroy: () => {
      destroyed = true; // a language still in flight must not touch a dead view
      view.destroy();
    },
  };
}

/** Replay a macro op list `times` times from the current cursor (Plan 30/31). */
function replayOps(view: EditorView, ops: readonly MacroOp[], times: number): void {
  if (ops.length === 0) return;
  for (let t = 0; t < times; t++) {
    for (const op of ops) {
      const cur = view.state.selection.main.head;
      if (op.kind === "insert") {
        view.dispatch({
          changes: { from: cur, insert: op.text },
          selection: { anchor: cur + op.text.length },
        });
      } else if (op.kind === "delete") {
        const from = op.dir === "backward" ? Math.max(0, cur - op.count) : cur;
        const to = op.dir === "backward" ? cur : Math.min(view.state.doc.length, cur + op.count);
        if (from !== to) view.dispatch({ changes: { from, to } });
      } else if (op.kind === "move") {
        applyMove(view, op.by);
      } else {
        applyLineOpToView(view, op.op);
      }
    }
  }
}

/** One named line operation over the selection's line range (Plan 27 semantics). */
function applyLineOpToView(view: EditorView, op: LineOp): void {
  const { state } = view;
  const sel = state.selection.main;
  // Empty selection → the whole document; otherwise the covered lines.
  const a = sel.empty ? 1 : state.doc.lineAt(sel.from).number;
  const b = sel.empty ? state.doc.lines : state.doc.lineAt(sel.to).number;
  const lines: string[] = [];
  for (let n = a; n <= b; n++) lines.push(state.doc.line(n).text);
  const out = applyLineOp(lines, op);
  view.dispatch({
    changes: { from: state.doc.line(a).from, to: state.doc.line(b).to, insert: out.join("\n") },
  });
}

/** Replay one recorded cursor move (Plan 30); plain steps ride CM commands. */
function applyMove(view: EditorView, by: MoveBy): void {
  const cmd: Partial<
    Record<Exclude<MoveBy, "line-start" | "line-end">, (v: EditorView) => boolean>
  > = {
    left: cursorCharLeft,
    right: cursorCharRight,
    up: cursorLineUp,
    down: cursorLineDown,
  };
  if (by === "line-start" || by === "line-end") {
    const line = view.state.doc.lineAt(view.state.selection.main.head);
    view.dispatch({
      selection: { anchor: by === "line-start" ? line.from : line.to },
      scrollIntoView: true,
    });
    return;
  }
  cmd[by]?.(view);
}

function buildState(
  text: string,
  lang: LanguageSupport | null,
  langConf: Compartment,
  wrapConf: Compartment,
  fontConf: Compartment,
  zoom: { size: number },
  wrap: boolean,
  onDocChanged: () => void,
  tools: {
    capture(tr: Transaction): void;
    captureMove(tr: Transaction): void;
    queueOverview(): void;
    minimapOn(): boolean;
  },
): EditorState {
  return EditorState.create({
    doc: text,
    extensions: [
      lineNumbers(),
      highlightActiveLine(),
      history(),
      foldGutter(),
      drawSelection(),
      indentOnInput(),
      bracketMatching(),
      rectangularSelection(),
      crosshairCursor(),
      wrapConf.of(wrap ? EditorView.lineWrapping : []),
      highlightSelectionMatches(),
      bookmarksPos,
      bookmarkDeco,
      bookmarkGutter,
      keymap.of([
        { key: "F2", run: nextBookmarkCmd },
        { key: "Shift-F2", run: prevBookmarkCmd },
        { key: "Ctrl-F2", run: toggleBookmarkCmd },
        ...defaultKeymap,
        ...historyKeymap,
        ...searchKeymap,
        indentWithTab,
      ]),
      search(),
      // Ctrl+wheel font zoom (Plan 27): intercept before the browser page-zoom.
      EditorView.domEventHandlers({
        wheel: (event, view) => {
          if (!event.ctrlKey) return false;
          event.preventDefault();
          zoom.size = clampFontSize(zoom.size + (event.deltaY < 0 ? 1 : -1));
          view.dispatch({
            effects: fontConf.reconfigure(fontTheme(zoom.size)),
          });
          return true;
        },
      }),
      // Language (may be none → PlainText) and the default highlight style coexist:
      // `lang-*` packages only emit tags, they carry no style, so the style must
      // always be present (spec §8).
      langConf.of(lang ?? []),
      fontConf.of(fontTheme(zoom.size)),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      EditorView.updateListener.of((u) => {
        // Macro capture (Plan 29/30): fold user edits + cursor moves into ops.
        // Not gated on u.docChanged — a pure cursor move must be captured too.
        for (const tr of u.transactions) {
          if (tr.isUserEvent("input.move") && !tr.docChanged) {
            tools.captureMove(tr);
            continue;
          }
          if (!u.docChanged || !tr.docChanged) continue;
          if (
            tr.isUserEvent("input.type") ||
            tr.isUserEvent("input.replace") ||
            tr.isUserEvent("delete.backward") ||
            tr.isUserEvent("delete.forward")
          ) {
            tools.capture(tr);
          }
        }
        if (u.docChanged) onDocChanged();
        if (u.docChanged || u.viewportChanged || u.geometryChanged) {
          if (tools.minimapOn()) tools.queueOverview();
        }
      }),
    ],
  });
}
