//! Edit-mode editor (spec §4.3 / §8, ≤200MB path). `openEditable` pulls the
//! whole document through `doc_chunk` and mounts a CodeMirror 6 `EditorView`.
//! Exposes a small handle the tab/status-bar layer drives; no IPC knowledge
//! lives here beyond `loadAllText`. EOL is a save-time concern carried on the
//! handle (the shell owns line-ending write-back), seeded from the core's
//! detected style so a CRLF file is not silently rewritten to LF (spec §4.4).

import { EditorState, Compartment } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  drawSelection,
  rectangularSelection,
  crosshairCursor,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { searchKeymap, search, openSearchPanel } from "@codemirror/search";
import {
  bracketMatching,
  foldGutter,
  indentOnInput,
  syntaxHighlighting,
  defaultHighlightStyle,
  type LanguageSupport,
} from "@codemirror/language";
import { loadAllText, type DocInfo } from "./ipc";
import { modeIdForExt, resolveLanguage, type ModeChoice } from "./language";

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
  setMode(mode: ModeChoice | null): void;
  setEol(eol: string): void;
  eol(): string;
  focus(): void;
  openSearch(): void;
  /** Scroll a 1-based source line into view and place the cursor there (CanPanel §7.4). */
  revealLine(line1: number): void;
  destroy(): void;
}

export interface OpenEditableOpts {
  parent: HTMLElement;
  info: DocInfo;
  /** File extension (no dot) used to pick the starting syntax mode. */
  ext: string;
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

  const lang = await resolveLanguage(modeIdForExt(opts.ext));
  const view = new EditorView({
    state: buildState(text, lang, langConf, onDocChanged),
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
    setMode: (mode: ModeChoice | null) => {
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
      EditorView.lineWrapping,
      keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
      search(),
      // Language (may be none → PlainText) and the default highlight style coexist:
      // `lang-*` packages only emit tags, they carry no style, so the style must
      // always be present (spec §8).
      langConf.of(lang ?? []),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) onDocChanged();
      }),
    ],
  });
}
