//! Edit-mode editor (spec §4.3 / §8, ≤200MB path). `openEditable` pulls the
//! whole document through `doc_chunk` and mounts a CodeMirror 6 `EditorView`.
//! Exposes a small handle the tab/status-bar layer drives; no IPC knowledge
//! lives here beyond `loadAllText`. EOL is a save-time concern carried on the
//! handle (the shell owns line-ending write-back), seeded from the core's
//! detected style so a CRLF file is not silently rewritten to LF (spec §4.4).

import { EditorState, Compartment, StateEffect, StateField } from "@codemirror/state";
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
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
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
    ),
    parent: opts.parent,
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
    destroy: () => {
      destroyed = true; // a language still in flight must not touch a dead view
      view.destroy();
    },
  };
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
        if (u.docChanged) onDocChanged();
      }),
    ],
  });
}
