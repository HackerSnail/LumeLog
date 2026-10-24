import { describe, it, expect } from "vitest";
import { FrameStore } from "./canpanel";
import type { FrameJson } from "./ipc";

// Only the headless, DOM-free half of CanPanel is unit-tested here: the frame
// store's batch-append contract (what `canParse`'s `onBatch` feeds) and clear.
// Column/virtual-scroll rendering is exercised under `tauri dev` (Task 7).
function frame(lineNumber: number): FrameJson {
  return {
    lineNumber,
    channel: 1,
    canId: 0x100 + lineNumber,
    canIdHex: "0x100",
    dlc: 8,
    dataHex: "DEADBEEF",
    isRemote: false,
    isFd: false,
    errorFrame: false,
    signals: [],
  };
}

describe("FrameStore (spec §7.4 batched frame accumulation)", () => {
  it("frame_store_appends_in_order: two pushes keep insert order, length grows", () => {
    const s = new FrameStore();
    s.push([frame(1), frame(2)]);
    s.push([frame(3)]);
    expect(s.length).toBe(3);
    expect(s.all().map((f) => f.lineNumber)).toEqual([1, 2, 3]);
  });

  it("frame_store_clear: empties the store", () => {
    const s = new FrameStore();
    s.push([frame(1), frame(2)]);
    s.clear();
    expect(s.length).toBe(0);
    expect(s.all()).toEqual([]);
  });

  it("frame_store_revision: bumps on every mutation, so the panel can skip re-filtering on scroll", () => {
    const s = new FrameStore();
    const r0 = s.revision;
    s.push([frame(1)]);
    expect(s.revision).toBe(r0 + 1);
    s.push([frame(2), frame(3)]);
    expect(s.revision).toBe(r0 + 2);
    s.clear();
    expect(s.revision).toBe(r0 + 3);
    // read-only access never advances the revision
    const before = s.revision;
    s.all();
    void s.length;
    expect(s.revision).toBe(before);
  });
});
