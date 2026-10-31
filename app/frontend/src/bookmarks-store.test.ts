import { describe, expect, it } from "vitest";
import {
  parseBookmarkStore,
  serializeBookmarkStore,
  BOOKMARKS_PER_DOC_MAX,
} from "./bookmarks-store";

describe("parseBookmarkStore", () => {
  it("returns {} for null/corrupt input", () => {
    expect(parseBookmarkStore(null)).toEqual({});
    expect(parseBookmarkStore("{oops")).toEqual({});
    expect(parseBookmarkStore("42")).toEqual({});
  });

  it("dedupes, sorts, drops non-positive and empty docs", () => {
    const json = JSON.stringify({
      "a.log": [9, 2, 2, 1, 0, -3, 5.5],
      "": [1],
      "b.log": "nope",
      "c.log": [],
    });
    expect(parseBookmarkStore(json)).toEqual({ "a.log": [1, 2, 9] });
  });

  it("caps lines per document", () => {
    const json = JSON.stringify({
      "a.log": Array.from({ length: BOOKMARKS_PER_DOC_MAX + 10 }, (_, i) => i + 1),
    });
    expect(parseBookmarkStore(json)["a.log"]).toHaveLength(BOOKMARKS_PER_DOC_MAX);
  });

  it("round-trips through serialize", () => {
    const store = { "a.log": [1, 5, 9], "b.log": [3] };
    expect(parseBookmarkStore(serializeBookmarkStore(store))).toEqual(store);
  });
});
