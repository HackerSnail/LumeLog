import { describe, it, expect } from "vitest";
import { pickTimelineInput } from "./query";

// spec §2: a filled extract regex selects the TIME axis; otherwise a line range
// (both sides, positive ints, from<=to) or a no-op. Returns `error` not throw.
describe("pickTimelineInput (M3 timeline axis, pure)", () => {
  it("extract_present_selects_time", () => {
    const s = pickTimelineInput("(?<ts>.*)", "iso", "1", "2", true);
    expect(s.time).toBeDefined();
    expect(s.time?.extractPattern).toBe("(?<ts>.*)");
    expect(s.time?.keepWhenNoTs).toBe(true);
    expect(s.lineFrom).toBeUndefined();
    expect(s.error).toBeUndefined();
  });

  it("time_allows_single_side_open_bound", () => {
    const s = pickTimelineInput("(?<ts>.*)", "epoch_ms", "", "5", false);
    expect(s.time?.from).toBeUndefined();
    expect(s.time?.to).toBe("5");
    expect(s.error).toBeUndefined();
  });

  it("extract_blank_sends_line_range", () => {
    const s = pickTimelineInput("", "iso", "3", "5", false);
    expect(s.time).toBeUndefined();
    expect(s.lineFrom).toBe(3);
    expect(s.lineTo).toBe(5);
  });

  it("all_blank_is_noop", () => {
    expect(pickTimelineInput("", "iso", "", "", false)).toEqual({});
  });

  it("line_range_requires_both_sides", () => {
    const s = pickTimelineInput("", "iso", "5", "", false);
    expect(s.error).toBeTruthy();
  });

  it("line_range_non_integer_errors", () => {
    expect(pickTimelineInput("", "iso", "a", "5", false).error).toBeTruthy();
    expect(pickTimelineInput("", "iso", "1.5", "5", false).error).toBeTruthy();
  });

  it("line_range_reversed_errors", () => {
    expect(pickTimelineInput("", "iso", "9", "2", false).error).toBeTruthy();
  });
});
