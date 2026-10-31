// @vitest-environment jsdom
import { beforeAll, describe, expect, it, vi } from "vitest";
import { language } from "@codemirror/language";
import type { DocInfo } from "./ipc";

vi.mock("./ipc", () => ({
  loadAllText: async () => ({ text: "b\na", invalidCount: 0 }),
}));

const { openEditable, bookmarkPositions } = await import("./editor");
const { resolveLanguage } = await import("./language");

beforeAll(() => {
  // jsdom lacks Range.getClientRects; `highlightSelectionMatches` needs it.
  (Range.prototype as unknown as { getClientRects: () => DOMRectList }).getClientRects =
    () => [] as unknown as DOMRectList;
});

function info(): DocInfo {
  return {
    docId: 1,
    size: 4,
    mtimeMillis: 0,
    encoding: "UTF-8",
    eol: "lf",
    mode: "edit",
    lineCountEstimate: 2,
    totalLines: 2,
  } as DocInfo;
}

async function mount(ext: string) {
  const parent = document.createElement("div");
  document.body.append(parent);
  const handle = await openEditable({ parent, info: info(), ext });
  return handle;
}

/** Let the pending language `import()` + reconfigure dispatch settle. */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 0));
}

describe("EditorHandle parity commands (Plan 27)", () => {
  it("lineOp sorts the whole document when nothing is selected", async () => {
    const handle = await mount("txt");
    handle.lineOp("sort-asc");
    expect(handle.getText()).toBe("a\nb");
    handle.lineOp("reverse");
    expect(handle.getText()).toBe("b\na");
    handle.lineOp("dedupe");
    expect(handle.getText()).toBe("b\na");
    handle.destroy();
  });

  it("toggles word wrap", async () => {
    const handle = await mount("txt");
    expect(handle.isWrap()).toBe(false);
    handle.setWrap(true);
    expect(handle.isWrap()).toBe(true);
    handle.setWrap(false);
    expect(handle.isWrap()).toBe(false);
    handle.destroy();
  });

  it("bookmarks the cursor line and walks with wraparound", async () => {
    const handle = await mount("txt"); // doc "b\na"
    handle.toggleBookmark();
    expect(bookmarkPositions(handle.view)).toEqual([0]);

    // only bookmark: next/prev both wrap back onto itself
    expect(handle.nextBookmark()).toBe(true);
    expect(handle.view.state.selection.main.head).toBe(0);

    // cursor to line 2 → bookmark it; prev walks back to line 1
    handle.view.dispatch({ selection: { anchor: 2 } });
    handle.toggleBookmark();
    expect(bookmarkPositions(handle.view)).toEqual([0, 2]);
    expect(handle.prevBookmark()).toBe(true);
    expect(handle.view.state.selection.main.head).toBe(0);

    handle.clearBookmarks();
    expect(bookmarkPositions(handle.view)).toEqual([]);
    expect(handle.nextBookmark()).toBe(false);
    handle.destroy();
  });

  it("restores bookmarks by line and reports changes (Plan 38)", async () => {
    const changes: number[][] = [];
    const handle = await openEditable({
      parent: document.createElement("div"),
      info: info(),
      ext: "txt",
      onBookmarksChanged: (lines) => changes.push(lines),
    });
    document.body.append(handle.root);

    // out-of-range lines are dropped; the rest land on line starts
    handle.loadBookmarkLines([9, 2, 1]);
    expect(bookmarkPositions(handle.view)).toEqual([0, 2]);
    expect(handle.bookmarkLines()).toEqual([1, 2]);

    handle.view.dispatch({ selection: { anchor: 2 } });
    handle.toggleBookmark();
    expect(changes).toEqual([[1]]);
    handle.clearBookmarks();
    expect(changes).toEqual([[1], []]);
    handle.destroy();
  });
});

describe("EditorHandle tools (Plan 29)", () => {
  it("records and replays a macro", async () => {
    const handle = await mount("txt"); // doc "b\na"
    expect(handle.startRecording()).toBe(true);
    expect(handle.startRecording()).toBe(false); // already recording
    // type "X" at the cursor (line 1 start)
    handle.view.dispatch({ changes: { from: 0, insert: "X" }, userEvent: "input.type" });
    expect(handle.isRecording()).toBe(true);
    expect(handle.stopRecording()).toBeGreaterThan(0);
    expect(handle.isRecording()).toBe(false);

    // replay at line 2 start: "X" is inserted there
    handle.view.dispatch({ selection: { anchor: 2 } });
    handle.playMacro(1);
    expect(handle.getText()).toContain("X");

    // a second play with an empty macro is a no-op
    handle.playMacro(1);
    handle.destroy();
  });

  it("records moves, delete direction and line ops (Plan 30)", async () => {
    const handle = await mount("txt"); // doc "b\na"
    handle.view.dispatch({ selection: { anchor: 0 } });
    expect(handle.startRecording()).toBe(true);

    // type "X" at 0 (real typing carries the selection along → head 1)
    handle.view.dispatch({
      changes: { from: 0, insert: "X" },
      selection: { anchor: 1 },
      userEvent: "input.type",
    });
    // ArrowLeft: head 1 → 0 (same line, col -1)
    handle.view.dispatch({ selection: { anchor: 0 }, userEvent: "input.move" });
    // Delete key removes "X" forward
    handle.view.dispatch({ changes: { from: 0, to: 1 }, userEvent: "delete.forward" });
    // palette command rides handle.lineOp → recorded as a named action
    handle.lineOp("reverse"); // doc is "b\na" again after the forward delete
    expect(handle.stopRecording()).toBe(4);
    expect(handle.macroOps()).toHaveLength(4);

    // reset and replay from a clean doc
    handle.replaceText("b\na");
    handle.view.dispatch({ selection: { anchor: 0 } });
    handle.playMacro(1, handle.macroOps());
    // X inserted at 0 → left → forward delete of "X" → line reverse of ["b", "a"]
    expect(handle.getText()).toBe("a\nb");

    // backward deletes replay backwards, not forwards
    handle.replaceText("b\na");
    handle.startRecording();
    handle.view.dispatch({ selection: { anchor: 3 } });
    handle.view.dispatch({ changes: { from: 2, to: 3 }, userEvent: "delete.backward" });
    handle.stopRecording();
    handle.replaceText("b\na");
    handle.view.dispatch({ selection: { anchor: 3 } });
    handle.playMacro(1);
    expect(handle.getText()).toBe("b\n");
    handle.destroy();
  });

  it("runs a macro to the end of the file (Plan 31)", async () => {
    const handle = await mount("txt"); // doc "b\na", 2 lines
    // record a single "down" move
    handle.view.dispatch({ selection: { anchor: 0 } });
    handle.startRecording();
    handle.view.dispatch({ selection: { anchor: 3 }, userEvent: "input.move" });
    handle.stopRecording();

    handle.view.dispatch({ selection: { anchor: 0 } });
    // play 1: head 0 → 3 (line 2); play 2: no progress → stop
    expect(handle.playMacroToEnd()).toBe(2);
    expect(handle.view.state.selection.main.head).toBe(3);

    // no recorded macro → 0
    const empty = await mount("txt");
    expect(empty.playMacroToEnd()).toBe(0);
    empty.destroy();
    handle.destroy();
  });

  it("records selection extension and word delete via keymap (Plan 32)", async () => {
    const handle = await mount("txt");
    handle.replaceText("hello world");
    handle.view.dispatch({ selection: { anchor: 11 } });
    handle.startRecording();
    // Ctrl+Backspace rides the hooked keymap binding (Mod-Backspace)
    handle.view.contentDOM.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Backspace", ctrlKey: true, cancelable: true, bubbles: true }),
    );
    expect(handle.stopRecording()).toBe(1);
    expect(handle.macroOps()).toEqual([{ kind: "del-word", dir: "backward" }]);

    // replay on a fresh doc: deletes the word before the cursor
    handle.replaceText("hello world");
    handle.view.dispatch({ selection: { anchor: 11 } });
    handle.playMacro(1);
    expect(handle.getText()).toBe("hello ");
    handle.destroy();
  });

  it("records shift-selection and replays the replace pattern (Plan 32)", async () => {
    const handle = await mount("txt");
    handle.replaceText("hello world");
    handle.view.dispatch({ selection: { anchor: 0 } });
    handle.startRecording();
    // real arrow keys emit userEvent "select" (spec §0)
    handle.view.dispatch({ selection: { anchor: 0, head: 1 }, userEvent: "select" });
    handle.view.dispatch({ selection: { anchor: 0, head: 2 }, userEvent: "select" });
    // typing over the selection: one transaction, delete+insert ops
    handle.view.dispatch({
      changes: { from: 0, to: 2, insert: "X" },
      selection: { anchor: 1 },
      userEvent: "input.replace",
    });
    handle.stopRecording();
    expect(handle.macroOps()).toEqual([
      { kind: "move", by: "right", select: true },
      { kind: "move", by: "right", select: true },
      { kind: "delete", count: 2, dir: "forward" },
      { kind: "insert", text: "X" },
    ]);

    handle.replaceText("hello world");
    handle.view.dispatch({ selection: { anchor: 0 } });
    handle.playMacro(1);
    expect(handle.getText()).toBe("Xllo world");
    handle.destroy();
  });

  it("lists symbols for the current mode", async () => {
    const handle = await mount("py");
    await flush();
    // mock doc "b\na" has no symbols; the important bit is the shape + mode id
    expect(handle.listSymbols()).toEqual([]);
    handle.setMode("python");
    await flush();
    expect(handle.listSymbols()).toEqual([]);
    handle.destroy();
  });
});

describe("EditorHandle.setMode", () => {
  it("mounts the language for the file extension", async () => {
    const handle = await mount("py");
    expect(handle.view.state.facet(language)?.name).toBe("python");
    handle.destroy();
  });

  it("keeps the last mode when switches race", async () => {
    await resolveLanguage("python");
    await resolveLanguage("json");
    const handle = await mount("txt");
    expect(handle.view.state.facet(language)).toBeNull();

    handle.setMode("python");
    handle.setMode("json");
    await flush();
    expect(handle.view.state.facet(language)?.name).toBe("json");

    handle.setMode("python");
    await flush();
    expect(handle.view.state.facet(language)?.name).toBe("python");
    handle.destroy();
  });

  it("ignores a language that lands after destroy", async () => {
    await resolveLanguage("rust"); // already cached: the dispatch would land if unguarded
    const handle = await mount("txt");
    handle.setMode("rust");
    handle.destroy();
    await flush();
    expect(handle.view.state.facet(language)).toBeNull();
  });
});
