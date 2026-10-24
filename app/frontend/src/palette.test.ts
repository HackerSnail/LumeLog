import { describe, it, expect } from "vitest";
import { fuzzyScore, filterCommands, type Command } from "./palette";

const cmd = (title: string): Command => ({ id: title, title, run: () => {} });

describe("fuzzyScore", () => {
  it("scores an empty query as 0 for any text", () => {
    expect(fuzzyScore("", "Save file")).toBe(0);
    expect(fuzzyScore("", "")).toBe(0);
  });

  it("returns null when the query is not a subsequence", () => {
    expect(fuzzyScore("z", "Save file")).toBeNull();
    expect(fuzzyScore("sav", "Search")).toBeNull(); // no 'v' after 'a'
  });

  it("is case-insensitive", () => {
    expect(fuzzyScore("SAVE", "save file")).toBe(fuzzyScore("save", "save file"));
  });

  it("rewards a prefix + consecutive run over a scattered match", () => {
    const prefix = fuzzyScore("sav", "Save file")!;
    const scattered = fuzzyScore("sav", "Settings and view")!;
    expect(prefix).not.toBeNull();
    expect(scattered).not.toBeNull();
    expect(prefix).toBeGreaterThan(scattered);
  });
});

describe("filterCommands", () => {
  const all = [cmd("Search view"), cmd("Save file"), cmd("Reload")];

  it("empty query returns every command in the given order", () => {
    expect(filterCommands("", all).map((c) => c.title)).toEqual([
      "Search view",
      "Save file",
      "Reload",
    ]);
  });

  it("ranks the best fuzzy match first", () => {
    expect(filterCommands("sav", all).map((c) => c.title)[0]).toBe("Save file");
  });

  it("drops commands that do not match at all", () => {
    expect(filterCommands("zzz", all)).toEqual([]);
  });

  it("matches only the subset that contains the subsequence", () => {
    expect(filterCommands("ea", all).map((c) => c.title)).toEqual([
      "Search view",
      "Reload",
    ]);
  });
});
