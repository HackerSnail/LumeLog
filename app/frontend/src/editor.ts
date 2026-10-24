//! Edit-mode editor (spec §4.3 / §8, ≤200MB path). `openEditable` pulls the
//! whole document through `doc_chunk` and mounts a CodeMirror 6 `EditorView`.
//! Exposes a small handle the tab/status-bar layer drives; no IPC knowledge
//! lives here beyond `loadAllText`. EOL is a save-time concern (the shell owns
//! line-ending write-back), so it is carried on the handle, not the buffer.

import { EditorState } from "@codemirror/state";
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
} from "@codemirror/language";
import { loadAllText, type DocInfo } from "./ipc";
import { modeForExtension } from "./language";

export interface EditorHandle {
  readonly docId: number;
  readonly view: EditorView;
  getText(): string;
  isDirty(): boolean;
  markSaved(): void;
  setMode(ext: string): void;
  setEol(eol: string): void;
  eol(): string;
  focus(): void;
  openSearch(): void;
  destroy(): void;
}

export interface OpenEditableOpts {
  parent: HTMLElement;
  info: DocInfo;
  filename: string;
  eol: string;
}

/**
 * Load and mount `info` for editing. Fetches every block (async) before building
 * the view, so the first paint shows the full document. Returns a handle once
 * the editor is live.
 */
export async function openEditable(opts: OpenEditableOpts): Promise<EditorHandle> {
  const total = opts.info.totalLines ?? opts.info.lineCountEstimate;
  const { text } = await loadAllText(opts.info.docId, total);

  let dirty = false;
  let eol = opts.eol;
  // Only user edits flip dirty: the listener is installed after construction,
  // so the initial doc load never counts as a change.
  const onDocChanged = () => {
    dirty = true;
  };

  const view = new EditorView({
    state: buildState(text, opts, onDocChanged),
    parent: opts.parent,
  });

  return {
    docId: opts.info.docId,
    view,
    getText: () => view.state.doc.toString(),
    isDirty: () => dirty,
    markSaved: () => {
      dirty = false;
    },
    setMode: (ext: string) => {
      view.setState(buildState(view.state.doc.toString(), opts, onDocChanged, ext));
      dirty = true;
    },
    setEol: (next: string) => {
      eol = next;
      dirty = true;
    },
    eol: () => eol,
    focus: () => view.focus(),
    openSearch: () => openSearchPanel(view),
    destroy: () => view.destroy(),
  };
}

function buildState(
  text: string,
  opts: OpenEditableOpts,
  onDocChanged: () => void,
  modeExt?: string,
): EditorState {
  const ext = modeExt ?? opts.filename;
  const lang = modeForExtension(ext);
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
      lang ?? syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) onDocChanged();
      }),
    ],
  });
}
