import { describe, expect, it } from "vitest";
import {
  normalizeSession,
  parseSession,
  serializeSession,
  SESSION_DOCS_MAX,
} from "./session";

describe("normalizeSession", () => {
  it("dedupes in order, drops empties, caps at 50", () => {
    const paths = ["a.log", "b.log", "a.log", "", "c.log"];
    const s = normalizeSession({ paths, active: "b.log" });
    expect(s.paths).toEqual(["a.log", "b.log", "c.log"]);
    expect(s.active).toBe("b.log");

    const many = Array.from({ length: SESSION_DOCS_MAX + 10 }, (_, i) => `f${i}.log`);
    expect(normalizeSession({ paths: many, active: null }).paths).toHaveLength(SESSION_DOCS_MAX);
  });

  it("nulls an active path that is not open", () => {
    const s = normalizeSession({ paths: ["a.log"], active: "ghost.log" });
    expect(s.active).toBeNull();
  });
});

describe("parseSession", () => {
  it("returns an empty session for null/corrupt input", () => {
    expect(parseSession(null)).toEqual({ paths: [], active: null });
    expect(parseSession("{oops")).toEqual({ paths: [], active: null });
    expect(parseSession("42")).toEqual({ paths: [], active: null });
    expect(parseSession(JSON.stringify({ paths: "nope", active: 1 }))).toEqual({
      paths: [],
      active: null,
    });
  });

  it("round-trips through serialize", () => {
    const s = { paths: ["a.log", "b.log"], active: "b.log" };
    expect(parseSession(serializeSession(s))).toEqual(s);
  });
});
