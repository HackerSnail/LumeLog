//! JSON/XML/SQL formatters (Plan 42): pure, DOM-free except XML's DOMParser.
//! All functions return `{ok:true, text}` / `{ok:false, error}` (SQL never
//! fails — it is a best-effort tokenizer) so the caller can toast and keep
//! the document untouched.

export type FormatResult = { ok: true; text: string } | { ok: false; error: string };

const ok = (text: string): FormatResult => ({ ok: true, text });
const err = (error: string): FormatResult => ({ ok: false, error });

// ---- JSON ---------------------------------------------------------------------

export function formatJson(text: string): FormatResult {
  try {
    return ok(JSON.stringify(JSON.parse(text), null, 2));
  } catch (e) {
    return err(`JSON 无效：${(e as Error).message}`);
  }
}

export function minifyJson(text: string): FormatResult {
  try {
    return ok(JSON.stringify(JSON.parse(text)));
  } catch (e) {
    return err(`JSON 无效：${(e as Error).message}`);
  }
}

// ---- XML ----------------------------------------------------------------------

/** Escape for element text / attribute values. */
function xmlEscape(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function xmlAttrEscape(s: string): string {
  return xmlEscape(s).replace(/"/g, "&quot;");
}

function serializeNode(node: Node, indent: string, out: string[]): void {
  const pad = `${indent}  `;
  switch (node.nodeType) {
    case Node.ELEMENT_NODE: {
      const el = node as Element;
      const attrs = [...el.attributes].map((a) => ` ${a.name}="${xmlAttrEscape(a.value)}"`).join("");
      const children = [...el.childNodes].filter(
        (c) => !(c.nodeType === Node.TEXT_NODE && c.textContent?.trim() === ""),
      );
      const name = el.nodeName;
      if (children.length === 0) {
        out.push(`${indent}<${name}${attrs}/>\n`);
        return;
      }
      // A single text child stays inline: <tag>text</tag>.
      if (children.length === 1 && children[0].nodeType === Node.TEXT_NODE) {
        out.push(
          `${indent}<${name}${attrs}>${xmlEscape((children[0].textContent ?? "").trim())}</${name}>\n`,
        );
        return;
      }
      out.push(`${indent}<${name}${attrs}>\n`);
      for (const c of children) serializeNode(c, pad, out);
      out.push(`${indent}</${name}>\n`);
      return;
    }
    case Node.TEXT_NODE:
      out.push(`${indent}${xmlEscape(node.textContent?.trim() ?? "")}\n`);
      return;
    case Node.COMMENT_NODE:
      out.push(`${indent}<!--${node.textContent ?? ""}-->\n`);
      return;
    case Node.CDATA_SECTION_NODE:
      out.push(`${indent}<![CDATA[${node.textContent ?? ""}]]>\n`);
      return;
    case Node.PROCESSING_INSTRUCTION_NODE:
      out.push(`${indent}<?${(node as ProcessingInstruction).target} ${node.textContent ?? ""}?>\n`);
      return;
    case Node.DOCUMENT_TYPE_NODE:
      out.push(`${indent}<!DOCTYPE ${node.nodeName}>\n`);
      return;
    default:
      return;
  }
}

export function validateXml(text: string): FormatResult {
  if (typeof DOMParser === "undefined") return err("当前环境无 DOMParser");
  const doc = new DOMParser().parseFromString(text, "application/xml");
  const bad = doc.querySelector("parsererror");
  if (bad) return err(`XML 无效：${bad.textContent?.trim() ?? "解析失败"}`);
  return ok("");
}

export function formatXml(text: string): FormatResult {
  const v = validateXml(text);
  if (!v.ok) return v;
  const doc = new DOMParser().parseFromString(text, "application/xml");
  const out: string[] = [];
  for (const child of [...doc.childNodes]) serializeNode(child, "", out);
  return ok(out.join("").trimEnd());
}

// ---- SQL ----------------------------------------------------------------------

const SQL_MAJOR =
  /^(SELECT|FROM|WHERE|GROUP\s+BY|ORDER\s+BY|HAVING|LIMIT|OFFSET|VALUES|SET|INSERT\s+INTO|UPDATE|DELETE\s+FROM|UNION(\s+ALL)?|LEFT\s+(OUTER\s+)?JOIN|RIGHT\s+(OUTER\s+)?JOIN|FULL\s+(OUTER\s+)?JOIN|INNER\s+JOIN|CROSS\s+JOIN|JOIN)$/i;

const SQL_KEYWORDS = new Set(
  ("SELECT,DISTINCT,ALL,FROM,AS,WHERE,AND,OR,NOT,NULL,IS,IN,LIKE,BETWEEN,EXISTS," +
    "JOIN,LEFT,RIGHT,FULL,INNER,OUTER,CROSS,ON,ORDER,GROUP,BY,HAVING,LIMIT,OFFSET," +
    "INSERT,INTO,VALUES,UPDATE,SET,DELETE,CREATE,TABLE,VIEW,INDEX,PRIMARY,KEY,FOREIGN," +
    "REFERENCES,UNION,CASE,WHEN,THEN,ELSE,END,ASC,DESC,COUNT,SUM,AVG,MIN,MAX")
    .split(",")
    .map((k) => k),
);

/** Best-effort SQL pretty-print: never fails. Quoted strings and comments
 *  keep their case/spacing verbatim; keywords outside them uppercase; each
 *  major clause and comma-separated list item starts a new line, indented by
 *  parenthesis depth. */
export function formatSql(text: string): string {
  // Scan into pieces: code (whitespace-collapsed, keywords cased), literals
  // and comments verbatim.
  const pieces: string[] = [];
  let i = 0;
  let code = "";
  const flushCode = (): void => {
    if (code.trim() === "") {
      code = "";
      return;
    }
    const spaced = code.replace(/\s+/g, " ");
    pieces.push(spaced.replace(/\b[A-Za-z_][A-Za-z0-9_]*\b/g, (w) =>
      SQL_KEYWORDS.has(w.toUpperCase()) ? w.toUpperCase() : w));
    code = "";
  };
  while (i < text.length) {
    const ch = text[i];
    if (ch === "'" || ch === '"') {
      flushCode();
      let j = i + 1;
      while (j < text.length) {
        if (text[j] === ch) {
          if (text[j + 1] === ch) j += 2; // doubled-quote escape
          else break;
        } else j++;
      }
      pieces.push(text.slice(i, Math.min(j + 1, text.length)));
      i = j + 1;
      continue;
    }
    if (ch === "-" && text[i + 1] === "-") {
      flushCode();
      const stop = text.indexOf("\n", i);
      pieces.push(text.slice(i, stop === -1 ? text.length : stop));
      i = stop === -1 ? text.length : stop;
      continue;
    }
    if (ch === "/" && text[i + 1] === "*") {
      flushCode();
      const end = text.indexOf("*/", i + 2);
      const stop = end === -1 ? text.length : end + 2;
      pieces.push(text.slice(i, stop));
      i = stop;
      continue;
    }
    code += ch;
    i++;
  }
  flushCode();

  // Re-tokenize code pieces: words AND punctuation (, ( )) as separate tokens
  // (literals/comments are atomic). Plain whitespace splitting would glue "a,"
  // into one token and commas would never break lines.
  type Tok = { t: string; atomic: boolean };
  const toks: Tok[] = [];
  for (const p of pieces) {
    if (/^['"]/.test(p) || /^(--|\/\*)/.test(p)) toks.push({ t: p, atomic: true });
    else
      for (const w of p.trim().match(/[(),]|[^\s,()]+/g) ?? []) {
        if (w !== "") toks.push({ t: w, atomic: false });
      }
  }

  const isClause = (s: string): boolean => SQL_MAJOR.test(s.trim());
  const lines: string[] = [];
  let line = "";
  let depth = 0;
  const push = (): void => {
    if (line.trim() !== "") lines.push(`${"  ".repeat(depth)}${line.trim()}`);
    line = "";
  };
  for (let k = 0; k < toks.length; k++) {
    const { t, atomic } = toks[k];
    // Clause starts: match up to three words ("LEFT OUTER JOIN").
    if (!atomic) {
      const three = [t, toks[k + 1]?.t, toks[k + 2]?.t].join(" ");
      const two = [t, toks[k + 1]?.t].join(" ");
      if (isClause(three)) {
        push();
        line = `${three} `;
        k += 2;
        continue;
      }
      if (isClause(two)) {
        push();
        line = `${two} `;
        k += 1;
        continue;
      }
      if (isClause(t)) {
        push();
        line = `${t} `;
        continue;
      }
      if (t === "(") {
        line = `${line.trimEnd()} (`;
        push(); // "FROM (" keeps its own (unindented) line
        depth++;
        continue;
      }
      if (t === ")") {
        push();
        depth = Math.max(0, depth - 1);
        line = ")";
        continue;
      }
      if (t === ",") {
        line = `${line.trimEnd()},`;
        push();
        continue;
      }
    }
    line = `${line.trimEnd()} ${t}`;
  }
  push();
  return lines.join("\n");
}
