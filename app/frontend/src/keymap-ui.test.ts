// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";
import { openKeymapModal } from "./keymap-ui";

const actions = [
  { id: "open" as const, title: "打开文件…" },
  { id: "save" as const, title: "保存" },
];

function press(key: string, mods: Partial<{ ctrl: boolean; alt: boolean; shift: boolean; meta: boolean }> = {}) {
  document.dispatchEvent(
    new KeyboardEvent("keydown", {
      key,
      ctrlKey: mods.ctrl ?? false,
      altKey: mods.alt ?? false,
      shiftKey: mods.shift ?? false,
      metaKey: mods.meta ?? false,
      bubbles: true,
      cancelable: true,
    }),
  );
}

const flush = () => new Promise((r) => setTimeout(r, 0));

function row(id: string): HTMLElement {
  return document.querySelector(`[data-action="${id}"]`) as HTMLElement;
}

describe("keymap modal", () => {
  it("records a valid key and rejects conflicts / bare keys", async () => {
    const onSet = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    openKeymapModal({
      actions,
      bindings: { open: "Ctrl+O", save: "Ctrl+S" },
      onSet,
      onResetAll: vi.fn().mockResolvedValue(undefined),
      onClose,
    });
    (row("save").querySelector(".keymap-record") as HTMLButtonElement).click();

    press("o", { ctrl: true }); // owned by open
    expect(row("save").querySelector(".keymap-error")!.textContent).toContain("打开文件");
    expect(onSet).not.toHaveBeenCalled();

    press("k"); // bare key would hijack typing
    expect(row("save").querySelector(".keymap-error")!.textContent).toContain("Ctrl");

    press("k", { alt: true });
    await flush();
    expect(onSet).toHaveBeenCalledWith("save", "Alt+K");
    expect(row("save").querySelector(".keymap-accel")!.textContent).toBe("Alt+K");

    (document.querySelector(".keymap-close") as HTMLButtonElement).click();
    expect(onClose).toHaveBeenCalledWith(true);
    expect(document.querySelector(".keymap-modal")).toBeNull();
  });

  it("Escape cancels recording without persisting", async () => {
    const onSet = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();
    openKeymapModal({
      actions,
      bindings: { open: "Ctrl+O" },
      onSet,
      onResetAll: vi.fn().mockResolvedValue(undefined),
      onClose,
    });
    (row("open").querySelector(".keymap-record") as HTMLButtonElement).click();
    press("Escape");
    expect(onSet).not.toHaveBeenCalled();
    expect(document.querySelector(".keymap-modal")).not.toBeNull();

    (document.querySelector(".keymap-close") as HTMLButtonElement).click();
    expect(onClose).toHaveBeenCalledWith(false);
    expect(document.querySelector(".keymap-modal")).toBeNull();
  });
});
