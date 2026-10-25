// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SCRIPT_TEMPLATE, openScriptModal, type ScriptModalOpts } from "./script-ui";

const flush = () => new Promise((r) => setTimeout(r, 0));
const wait = async (): Promise<void> => {
  await flush();
  await flush();
};

beforeEach(() => {
  document.querySelectorAll(".script-modal").forEach((n) => n.remove());
  document.getElementById("app")?.removeAttribute("inert");
});

function baseOpts(over: Partial<ScriptModalOpts> = {}): ScriptModalOpts {
  return {
    hasDocument: () => true,
    defaultOutputBase: () => "doc",
    list: vi.fn().mockResolvedValue([
      { name: "a.js", sizeBytes: 10, modifiedMs: 0 },
      { name: "b.js", sizeBytes: 20, modifiedMs: 0 },
    ]),
    read: vi.fn().mockResolvedValue("function onLine(l) { return l; }\n"),
    write: vi.fn().mockResolvedValue(undefined),
    remove: vi.fn().mockResolvedValue(undefined),
    chooseTarget: vi.fn().mockResolvedValue("/tmp/out.txt"),
    run: vi.fn().mockResolvedValue(7),
    cancel: vi.fn().mockResolvedValue(undefined),
    confirm: vi.fn().mockResolvedValue(true),
    onClose: vi.fn(),
    ...over,
  };
}

const q = <T extends HTMLElement>(sel: string): T => document.querySelector(sel) as T;

async function closeModal(): Promise<void> {
  (q(".script-close") as HTMLButtonElement).click();
  await wait();
}

describe("script modal", () => {
  it("lists scripts and loads the clicked one", async () => {
    openScriptModal(baseOpts());
    await flush();
    expect(document.querySelectorAll(".script-row").length).toBe(2);
    (q('[data-name="b.js"]') as HTMLElement).click();
    await flush();
    expect(q<HTMLTextAreaElement>(".script-editor").value).toContain("onLine");
    await closeModal();
  });

  it("creates a script with the template and validates names", async () => {
    const opts = baseOpts();
    openScriptModal(opts);
    await flush();
    const input = q<HTMLInputElement>(".script-new-name");
    input.value = "bad name.js";
    (q(".script-new") as HTMLButtonElement).click();
    await wait();
    expect(opts.write).not.toHaveBeenCalled();
    expect(q(".script-status").textContent).toContain("仅允许");

    input.value = "fresh.js";
    (q(".script-new") as HTMLButtonElement).click();
    await wait();
    expect(opts.write).toHaveBeenCalledWith("fresh.js", SCRIPT_TEMPLATE);
    await closeModal();
  });

  it("saves the edited text of the selected script", async () => {
    const opts = baseOpts();
    openScriptModal(opts);
    await wait();
    (q('[data-name="a.js"]') as HTMLElement).click();
    await wait();
    const area = q<HTMLTextAreaElement>(".script-editor");
    area.value = "function onLine(){ return null; }";
    area.dispatchEvent(new Event("input"));
    (q(".script-save") as HTMLButtonElement).click();
    await wait();
    expect(opts.write).toHaveBeenCalledWith("a.js", "function onLine(){ return null; }");
    await closeModal();
  });

  it("runs with the editor code and streams console lines", async () => {
    const opts = baseOpts({
      hasDocument: () => false,
      run: vi.fn().mockImplementation(async (a) => {
        a.onConsole({ level: "log", text: "hello" });
        a.onProgress({ scanned: 10, written: 2 });
        a.onDone({
          scanned: 10,
          written: 2,
          dropped: 8,
          cancelled: false,
          truncatedConsole: false,
          error: null,
        });
        return 3;
      }),
    });
    openScriptModal(opts);
    await wait();
    (q(".script-run") as HTMLButtonElement).click();
    await wait();
    expect(q(".script-status").textContent).toContain("先打开一个文档");
    expect(opts.run).not.toHaveBeenCalled();

    await closeModal();
    openScriptModal(baseOpts({ run: opts.run }));
    await wait();
    (q('[data-name="a.js"]') as HTMLElement).click();
    await wait();
    (q(".script-run") as HTMLButtonElement).click();
    await wait();
    const run = opts.run as ReturnType<typeof vi.fn>;
    expect(run).toHaveBeenCalled();
    expect(run.mock.calls[0][0].targetPath).toBe("/tmp/out.txt");
    expect(q(".script-console").textContent).toContain("hello");
    await closeModal();
  });
});
