// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { ParserPanel, RowStore } from "./parserpanel";
import type { ParserRow } from "./ipc";

function rows(...spec: Array<[number, string[]]>): ParserRow[] {
  return spec.map(([line, cells]) => ({ line, cells }));
}

/** Flush the panel's rAF-coalesced paints: wait for a full rAF tick. */
function flush(): Promise<void> {
  return new Promise((resolve) =>
    requestAnimationFrame(() => setTimeout(resolve, 0)),
  );
}

function mount(opts = {
  onRowActivate: (_n: number) => {},
  onError: (e: string) => console.error("[panel]", e),
}): {
  host: HTMLElement;
  panel: ParserPanel;
} {
  const host = document.createElement("div");
  document.body.append(host);
  const panel = new ParserPanel(host, opts);
  return { host, panel };
}

describe("RowStore", () => {
  it("accumulates, clears and bumps revision", () => {
    const s = new RowStore();
    expect(s.length).toBe(0);
    s.push(rows([1, ["a"]]));
    s.push(rows([2, ["b"]], [3, ["c"]]));
    expect(s.length).toBe(3);
    const rev = s.revision;
    s.clear();
    expect(s.length).toBe(0);
    expect(s.revision).toBe(rev + 1);
  });
});

describe("ParserPanel", () => {
  it("renders dynamic column headers and pushed rows", async () => {
    const { host, panel } = mount();
    const store = new RowStore();
    panel.attachStore(store);
    panel.setColumns([
      { key: "time", label: "时间" },
      { key: "level", label: "级别" },
    ]);
    store.push(rows([1, ["12:00", "INFO"]], [2, ["12:01", "ERROR"]]));
    panel.render();
    await flush();

    const headers = Array.from(host.querySelectorAll(".can-hrow .can-cell")).map(
      (c) => c.textContent,
    );
    expect(headers).toEqual(["行号", "时间", "级别"]);
    const painted = Array.from(host.querySelectorAll(".can-row"));
    expect(painted.length).toBe(2);
    expect(painted[0]!.textContent).toContain("INFO");
    panel.destroy();
  });

  it("reports the source line on row click", async () => {
    const seen: number[] = [];
    const { host, panel } = mount({ onRowActivate: (n) => seen.push(n), onError: () => {} });
    const store = new RowStore();
    panel.attachStore(store);
    panel.setColumns([{ key: "k", label: "K" }]);
    store.push(rows([7, ["x"]]));
    panel.render();
    await flush();

    (host.querySelector(".can-row") as HTMLElement).click();
    expect(seen).toEqual([7]);
    panel.destroy();
  });

  it("keeps the status line text set via setStatus", async () => {
    const { host, panel } = mount();
    panel.setStatus("已解析 12 行 · 跳过 3 行");
    const status = host.querySelector(".can-status") as HTMLElement;
    expect(status.textContent).toBe("已解析 12 行 · 跳过 3 行");
    panel.destroy();
  });

  it("stops reacting after destroy", async () => {
    const { host, panel } = mount();
    const store = new RowStore();
    panel.attachStore(store);
    panel.setColumns([{ key: "k", label: "K" }]);
    panel.destroy();
    store.push(rows([1, ["a"]]));
    panel.render();
    await flush();
    expect(host.querySelectorAll(".can-row").length).toBe(0);
  });
});
