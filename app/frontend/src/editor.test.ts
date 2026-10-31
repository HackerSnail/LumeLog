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
