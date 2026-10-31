import { describe, expect, it } from "vitest";
import {
  applyReplace,
  buildReplaceRe,
  escapeRegex,
  findMatches,
  replacePreview,
} from "./replacetools";

describe("buildReplaceRe", () => {
  it("escapes literal metacharacters", () => {
    const re = buildReplaceRe("a.b(c)", "literal", true) as RegExp;
    expect("xa.b(c)y".replace(re, "Z")).toBe("xZy");
  });

  it("rejects empty find and bad regex", () => {
    expect(buildReplaceRe("", "literal", true)).toHaveProperty("error");
    expect(buildReplaceRe("([", "regex", true)).toHaveProperty("error");
  });
});

describe("findMatches", () => {
  it("reports line numbers and previews", () => {
    const re = buildReplaceRe("val", "literal", true) as RegExp;
    const m = findMatches("row val=1\nrow val=2\nno hit", re);
    expect(m.map((x) => x.line)).toEqual([1, 2]);
    expect(m[1].preview).toBe("row val=2");
  });

  it("skips zero-length matches without looping", () => {
    // "x*" on a text without x: every match is empty → none recorded, no hang.
    const re = buildReplaceRe("x*", "regex", true) as RegExp;
    expect(findMatches("bab", re).length).toBe(0);
  });

  it("respects case sensitivity", () => {
    const cs = buildReplaceRe("can", "literal", true) as RegExp;
    const ci = buildReplaceRe("can", "literal", false) as RegExp;
    expect(findMatches("CAN can", cs)).toHaveLength(1);
    expect(findMatches("CAN can", ci)).toHaveLength(2);
  });
});

describe("applyReplace", () => {
  it("keeps $ literal in literal mode", () => {
    const re = buildReplaceRe("x", "literal", true) as RegExp;
    expect(applyReplace("x", re, "$&$1", "literal")).toBe("$&$1");
  });

  it("expands group references in regex mode", () => {
    const re = buildReplaceRe("id=(\\d+)", "regex", true) as RegExp;
    expect(applyReplace("id=42", re, "[$1]", "regex")).toBe("[42]");
  });
});

describe("replacePreview", () => {
  it("combines find + replace with a full result", () => {
    const r = replacePreview("a,b\na,c", "a", "X", "literal", true);
    expect(r).toEqual({
      ok: true,
      matches: [
        { from: 0, to: 1, line: 1, preview: "a,b" },
        { from: 4, to: 5, line: 2, preview: "a,c" },
      ],
      result: "X,b\nX,c",
    });
  });

  it("propagates pattern errors", () => {
    const r = replacePreview("x", "([", "", "regex", true);
    expect("error" in r).toBe(true);
  });

  it("caps long line previews", () => {
    const long = "x".repeat(300);
    const r = replacePreview(long, "x", "y", "literal", true);
    expect(r.ok && r.matches[0].preview).toHaveLength(201);
  });
});

describe("escapeRegex", () => {
  it("escapes the metacharacter set", () => {
    expect(escapeRegex("a.b*c[d]e(f)g{}h$^i|j\\k")).toBe(
      "a\\.b\\*c\\[d\\]e\\(f\\)g\\{\\}h\\$\\^i\\|j\\\\k",
    );
  });
});
