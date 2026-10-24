import { describe, it, expect } from "vitest";
import { applyCanFilter, type CanFilter } from "./canfilter";
import type { FrameJson, SignalValueJson } from "./ipc";

// Fixed frames covering the filter's axes: an in-range/EngineData frame with a
// Speed signal, an extended frame with no msgName, and a timestamp-less frame.
function sig(name: string, physical: number): SignalValueJson {
  return { name, raw: physical, physical, unit: "degC" };
}
function frame(over: Partial<FrameJson> = {}): FrameJson {
  return {
    lineNumber: 1,
    channel: 1,
    canId: 0x1a0,
    canIdHex: "0x1A0",
    dlc: 8,
    dataHex: "DEADBEEF",
    isRemote: false,
    isFd: false,
    errorFrame: false,
    msgName: "EngineData",
    signals: [sig("Speed", 100), sig("CoolantTemp", 50)],
    ...over,
  };
}

const FRAMES: FrameJson[] = [
  frame({ lineNumber: 1, canId: 0x100, canIdHex: "0x100", timestampMs: 1500 }),
  frame({ lineNumber: 2, canId: 0x1ff, canIdHex: "0x1FF", timestampMs: 2000 }),
  frame({ lineNumber: 3, canId: 0x300, canIdHex: "0x300", msgName: undefined, signals: [], timestampMs: 5000 }),
  frame({ lineNumber: 4, canId: 0x150, canIdHex: "0x150", signals: [sig("Speed", 30)], timestampMs: undefined }),
];

describe("applyCanFilter (spec §7.4 view filter)", () => {
  it("no_filter_returns_all: empty filter keeps order & length", () => {
    const out = applyCanFilter(FRAMES, {});
    expect(out).toHaveLength(4);
    expect(out.map((f) => f.lineNumber)).toEqual([1, 2, 3, 4]);
  });

  it("id_range_inclusive: closed interval, reversed range yields empty", () => {
    const fwd = applyCanFilter(FRAMES, { idFrom: 0x100, idTo: 0x1ff });
    expect(fwd.map((f) => f.lineNumber)).toEqual([1, 2, 4]);
    // only a lower bound
    expect(applyCanFilter(FRAMES, { idFrom: 0x200 }).map((f) => f.lineNumber)).toEqual([3]);
    // only an upper bound
    expect(applyCanFilter(FRAMES, { idTo: 0x150 }).map((f) => f.lineNumber)).toEqual([1, 4]);
    // reversed range never matches, does not throw
    expect(applyCanFilter(FRAMES, { idFrom: 0x1ff, idTo: 0x100 })).toEqual([]);
  });

  it("msg_name_matches: exact name only, undefined msgName filtered out", () => {
    const out = applyCanFilter(FRAMES, { msgName: "EngineData" });
    expect(out.map((f) => f.lineNumber)).toEqual([1, 2, 4]);
  });

  it("signal_condition: named signal compared by op, == uses Object.is", () => {
    const gt = applyCanFilter(FRAMES, { signalName: "Speed", signalOp: ">", signalValue: 60 });
    expect(gt.map((f) => f.lineNumber)).toEqual([1, 2]);
    const eq = applyCanFilter(FRAMES, { signalName: "Speed", signalOp: "==", signalValue: 30 });
    expect(eq.map((f) => f.lineNumber)).toEqual([4]);
    // a frame lacking the named signal is dropped
    expect(applyCanFilter(FRAMES, { signalName: "Nope", signalOp: ">", signalValue: 0 })).toEqual([]);
  });

  it("time_window_drops_ts_none: closed interval excludes missing timestamps", () => {
    const out = applyCanFilter(FRAMES, { timeFromMs: 1000, timeToMs: 2000 });
    expect(out.map((f) => f.lineNumber)).toEqual([1, 2]);
  });

  it("combined_predicates_are_and", () => {
    const out = applyCanFilter(FRAMES, { msgName: "EngineData", idFrom: 0x100, idTo: 0x100 });
    expect(out.map((f) => f.lineNumber)).toEqual([1]);
  });

  it("does_not_mutate: input array and frames unchanged", () => {
    const before = structuredClone(FRAMES);
    const f: CanFilter = { idFrom: 0x100, signalName: "Speed", signalOp: ">", signalValue: 0 };
    applyCanFilter(FRAMES, f);
    expect(FRAMES).toEqual(before);
  });
});
