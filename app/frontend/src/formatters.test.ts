// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { formatJson, formatSql, formatXml, minifyJson, validateXml } from "./formatters";

describe("JSON", () => {
  it("formats and minifies", () => {
    expect(formatJson('{"a":1,"b":[2,3]}')).toEqual({
      ok: true,
      text: '{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ]\n}',
    });
    expect(minifyJson('{ "a" : 1 }')).toEqual({ ok: true, text: '{"a":1}' });
  });

  it("reports parse errors without position loss", () => {
    const r = formatJson('{"a":}');
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toContain("JSON 无效");
  });
});

describe("XML", () => {
  it("formats elements, attributes, comments and self-closing tags", () => {
    const r = formatXml('<root a="1" b="x&quot;y"><!--note--><child/><t> hi </t></root>');
    expect(r).toEqual({
      ok: true,
      text: [
        '<root a="1" b="x&quot;y">',
        "  <!--note-->",
        "  <child/>",
        "  <t>hi</t>",
        "</root>",
      ].join("\n"),
    });
  });

  it("keeps XML-special characters escaped", () => {
    const r = formatXml("<t>a &lt; b &amp; c</t>");
    expect(r.ok && r.text.includes("a &lt; b &amp; c")).toBe(true);
  });

  it("rejects malformed input", () => {
    expect(validateXml("<a><b></a>").ok).toBe(false);
    expect(formatXml("<a>").ok).toBe(false);
  });
});

describe("SQL", () => {
  it("uppercases keywords but preserves strings and comments", () => {
    const r = formatSql(
      "select a, b from t where a = 'keep lower From' -- note: select\n/* order by nothing */",
    );
    expect(r).toBe(
      [
        "SELECT a,",
        "b",
        "FROM t",
        "WHERE a = 'keep lower From' -- note: select /* order by nothing */",
      ].join("\n"),
    );
  });

  it("breaks clauses and indents by parenthesis depth", () => {
    const r = formatSql(
      "select x from (select y from t where y > 1) sub where x < 10 group by x order by x",
    );
    expect(r).toBe(
      [
        "SELECT x",
        "FROM (",
        "  SELECT y",
        "  FROM t",
        "  WHERE y > 1",
        ") sub",
        "WHERE x < 10",
        "GROUP BY x",
        "ORDER BY x",
      ].join("\n"),
    );
  });

  it("handles JOIN and doubled-quote escapes", () => {
    const r = formatSql("select * from a left join b on a.id = b.id where b.n = 'it''s'");
    expect(r).toContain("LEFT JOIN b");
    expect(r).toContain("'it''s'");
  });
});
