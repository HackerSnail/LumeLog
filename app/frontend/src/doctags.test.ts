import { describe, expect, it } from "vitest";
import {
  addTag,
  normalizeTag,
  parseTagStore,
  removeTag,
  serializeTagStore,
  TAG_MAX_LEN,
  TAGS_PER_DOC_MAX,
} from "./doctags";

describe("normalizeTag", () => {
  it("trims, rejects empty, truncates", () => {
    expect(normalizeTag("  台架  ")).toBe("台架");
    expect(normalizeTag("   ")).toBeNull();
    expect(normalizeTag("长".repeat(TAG_MAX_LEN + 5))).toHaveLength(TAG_MAX_LEN);
  });
});

describe("addTag / removeTag", () => {
  it("adds without mutating the input, dedupes, refuses full stores", () => {
    let store = {};
    const r1 = addTag(store, "a.log", " CAN ");
    expect(r1.tag).toBe("CAN");
    store = r1.store;
    expect(store).toEqual({ "a.log": ["CAN"] });
    expect(addTag(store, "a.log", "CAN").reason).toBe("dup");
    for (let i = 0; i < TAGS_PER_DOC_MAX - 1; i++) {
      store = addTag(store, "a.log", `t${i}`).store;
    }
    expect(addTag(store, "a.log", "再来一个").reason).toBe("full");
    expect(addTag(store, "a.log", "  ")).toEqual({ store, tag: null });
  });

  it("removes and drops empty documents", () => {
    let store = addTag(addTag({}, "a.log", "x").store, "a.log", "y").store;
    store = removeTag(store, "a.log", "x");
    expect(store).toEqual({ "a.log": ["y"] });
    store = removeTag(store, "a.log", "y");
    expect(store).toEqual({});
    expect(removeTag(store, "a.log", "y")).toEqual(store); // already gone
  });
});

describe("parseTagStore / serializeTagStore", () => {
  it("round-trips", () => {
    const store = addTag(addTag({}, "a.log", "HIL").store, "b.log", "GB2312").store;
    expect(parseTagStore(serializeTagStore(store))).toEqual(store);
  });

  it("tolerates corrupt input and drops bad entries", () => {
    expect(parseTagStore(null)).toEqual({});
    expect(parseTagStore("{oops")).toEqual({});
    expect(parseTagStore("42")).toEqual({});
    const json = JSON.stringify({
      "a.log": ["ok", "", 42, "ok", "已截".repeat(20)],
      "": ["no-key"],
      "b.log": "not-an-array",
    });
    expect(parseTagStore(json)).toEqual({
      "a.log": ["ok", "已截".repeat(TAG_MAX_LEN / 2)],
    });
  });

  it("caps tags per document", () => {
    const tags = Array.from({ length: TAGS_PER_DOC_MAX + 5 }, (_, i) => `t${i}`);
    const parsed = parseTagStore(JSON.stringify({ "a.log": tags }));
    expect(parsed["a.log"]).toHaveLength(TAGS_PER_DOC_MAX);
  });
});
