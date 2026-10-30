// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { language } from "@codemirror/language";
import type { DocInfo } from "./ipc";

vi.mock("./ipc", () => ({
  loadAllText: async () => ({ text: "a\nb\n", invalidCount: 0 }),
}));

const { openEditable } = await import("./editor");
const { resolveLanguage } = await import("./language");

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
