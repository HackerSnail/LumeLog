//! Syntax highlighting modes (spec §8). Every language lives in one registry:
//! `mode id -> { extensions, loader }`. `modeIdForExt` answers "which mode does
//! this file use" without loading anything (it backs the AUTO choice); the
//! language packages themselves are pulled in on demand by `resolveLanguage`,
//! so a file that never opens a Python file never pays for the Python grammar.
//!
//! Highlight colours: `log`/`ini` carry their own inline `streamHighlight`;
//! everything else only emits tags and is coloured by `defaultHighlightStyle`
//! (mounted with `fallback: true` in `editor.ts`), which is why adding a
//! language needs no new `HighlightStyle`.

import {
  HighlightStyle,
  StreamLanguage,
  syntaxHighlighting,
  LanguageSupport,
  type StreamParser,
} from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

/** Log-style highlighting: timestamps, levels, numbers, hex, strings. */
const logLanguage = StreamLanguage.define<{ inQuotes: boolean }>({
  startState: () => ({ inQuotes: false }),
  copyState: (s) => ({ inQuotes: s.inQuotes }),
  token(stream, state) {
    // Timestamp prefix only — consume just the timestamp, not the whole line,
    // so the level / numbers later on the same line still get coloured.
    if (
      stream.sol() &&
      stream.match(/^\[?\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}(?::\d{2})?(?:\.\d+)?\]?/)
    ) {
      return "className";
    }
    if (stream.eatSpace()) return null;
    if (state.inQuotes) {
      if (stream.match(/^[^"]*"?/)) {
        if (stream.string[stream.pos - 1] === '"') state.inQuotes = false;
      } else {
        stream.next();
      }
      return "string";
    }
    if (stream.match(/^"([^"]|"")*"/)) return "string";
    if (stream.eat('"')) {
      state.inQuotes = true;
      return "string";
    }
    if (stream.match(/^0[xX][0-9a-fA-F]+/)) return "number";
    if (stream.match(/^\w+/)) {
      const word = stream.current().toUpperCase();
      if (/^(TRACE|DEBUG|INFO|NOTICE|WARN|WARNING|ERROR|ERR|FATAL|CRITICAL)$/.test(word)) {
        return "keyword";
      }
      return null;
    }
    stream.next();
    return null;
  },
});

/** Minimal INI: `[sections]`, `key = value`, `#`/`;` comments. */
const iniLanguage = StreamLanguage.define({
  token(stream) {
    if (stream.sol()) {
      if (/^[#;]/.test(stream.string.slice(stream.pos))) {
        stream.skipToEnd();
        return "comment";
      }
      if (stream.match(/^\[[^\]]*\]/)) return "className";
      if (stream.match(/^[^=\s]+(?=\s*=)/)) return "propertyName";
    }
    if (stream.eatSpace()) return null;
    if (stream.eat("=")) return "operator";
    if (/[#;]/.test(stream.string[stream.pos])) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match(/^[^#;\s]+/)) return "string";
    stream.next();
    return null;
  },
});

/** Inline highlight style for the log / INI stream modes (no CSS needed). */
const streamHighlight = HighlightStyle.define([
  { tag: t.keyword, color: "#b25000", fontWeight: "bold" }, // log level
  { tag: t.className, color: "#708090" }, // timestamp / INI [section]
  { tag: t.number, color: "#1a6fb0" }, // number / hex
  { tag: t.string, color: "#0a7d33" }, // value / quoted string
  { tag: t.comment, color: "#8a8a8a", fontStyle: "italic" },
  { tag: t.propertyName, color: "#800080" }, // INI key
  { tag: t.operator, color: "#666" },
]);

/** The set of languages the status-bar switcher offers. */
export const AVAILABLE_MODES = [
  "auto",
  "plain",
  "log",
  "ini",
  "json",
  "c",
  "javascript",
  "typescript",
  "python",
  "html",
  "css",
  "xml",
  "markdown",
  "sql",
  "rust",
  "java",
  "yaml",
  "toml",
  "shell",
  "diff",
  "powershell",
] as const;
export type ModeChoice = (typeof AVAILABLE_MODES)[number];

/** Dropdown text — mode ids stay stable, these are only what the user reads. */
export const MODE_LABELS: Record<ModeChoice, string> = {
  auto: "Auto",
  plain: "Plain Text",
  log: "Log",
  ini: "INI",
  json: "JSON",
  c: "C/C++",
  javascript: "JavaScript",
  typescript: "TypeScript",
  python: "Python",
  html: "HTML",
  css: "CSS",
  xml: "XML",
  markdown: "Markdown",
  sql: "SQL",
  rust: "Rust",
  java: "Java",
  yaml: "YAML",
  toml: "TOML",
  shell: "Shell",
  diff: "Diff",
  powershell: "PowerShell",
};

/** Extension (no dot, lowercased) → mode id. Absent = PlainText. */
const EXT_MODES: Record<string, ModeChoice> = {
  log: "log",
  ini: "ini",
  cfg: "ini",
  conf: "ini",
  json: "json",
  c: "c",
  h: "c",
  cpp: "c",
  cc: "c",
  cxx: "c",
  hpp: "c",
  hh: "c",
  js: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  jsx: "javascript",
  ts: "typescript",
  tsx: "typescript",
  mts: "typescript",
  cts: "typescript",
  py: "python",
  pyw: "python",
  pyi: "python",
  html: "html",
  htm: "html",
  css: "css",
  xml: "xml",
  xsd: "xml",
  xsl: "xml",
  svg: "xml",
  plist: "xml",
  resx: "xml",
  md: "markdown",
  markdown: "markdown",
  sql: "sql",
  rs: "rust",
  java: "java",
  yaml: "yaml",
  yml: "yaml",
  toml: "toml",
  sh: "shell",
  bash: "shell",
  zsh: "shell",
  diff: "diff",
  patch: "diff",
  ps1: "powershell",
  psm1: "powershell",
  psd1: "powershell",
};

/**
 * Synchronous extension → mode lookup (drives AUTO). Returns null for anything
 * unknown, which the caller turns into PlainText — never throws.
 */
export function modeIdForExt(ext: string): ModeChoice | null {
  const e = (ext ?? "").trim().toLowerCase();
  return EXT_MODES[e] ?? null;
}

const loaded = new Map<ModeChoice, LanguageSupport | null>();
const inFlight = new Map<ModeChoice, Promise<LanguageSupport | null>>();

/**
 * Load (once) the language for a mode. `auto`/`plain`/null → null = PlainText.
 * Concurrent callers share a single import so a fast mode switch does not
 * fetch the same grammar twice.
 */
export async function resolveLanguage(mode: ModeChoice | null): Promise<LanguageSupport | null> {
  if (!mode || mode === "auto" || mode === "plain") return null;
  const done = loaded.get(mode);
  if (done !== undefined) return done;
  let p = inFlight.get(mode);
  if (!p) {
    p = loadLanguage(mode).then((lang) => {
      loaded.set(mode, lang);
      inFlight.delete(mode);
      return lang;
    });
    inFlight.set(mode, p);
  }
  return p;
}

async function loadLanguage(mode: ModeChoice): Promise<LanguageSupport | null> {
  switch (mode) {
    case "log":
      return new LanguageSupport(logLanguage, [syntaxHighlighting(streamHighlight)]);
    case "ini":
      return new LanguageSupport(iniLanguage, [syntaxHighlighting(streamHighlight)]);
    case "json": {
      const { json } = await import("@codemirror/lang-json");
      return json();
    }
    case "c": {
      const { cpp } = await import("@codemirror/lang-cpp");
      return cpp();
    }
    case "javascript": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript();
    }
    case "typescript": {
      const { javascript } = await import("@codemirror/lang-javascript");
      return javascript({ typescript: true, jsx: true });
    }
    case "python": {
      const { python } = await import("@codemirror/lang-python");
      return python();
    }
    case "html": {
      const { html } = await import("@codemirror/lang-html");
      return html();
    }
    case "css": {
      const { css } = await import("@codemirror/lang-css");
      return css();
    }
    case "xml": {
      const { xml } = await import("@codemirror/lang-xml");
      return xml();
    }
    case "markdown": {
      const { markdown } = await import("@codemirror/lang-markdown");
      return markdown();
    }
    case "sql": {
      const { sql } = await import("@codemirror/lang-sql");
      return sql();
    }
    case "rust": {
      const { rust } = await import("@codemirror/lang-rust");
      return rust();
    }
    case "java": {
      const { java } = await import("@codemirror/lang-java");
      return java();
    }
    case "yaml":
      return streamMode(import("@codemirror/legacy-modes/mode/yaml"), "yaml");
    case "toml":
      return streamMode(import("@codemirror/legacy-modes/mode/toml"), "toml");
    case "shell":
      return streamMode(import("@codemirror/legacy-modes/mode/shell"), "shell");
    case "diff":
      return streamMode(import("@codemirror/legacy-modes/mode/diff"), "diff");
    case "powershell":
      return streamMode(import("@codemirror/legacy-modes/mode/powershell"), "powerShell");
    default:
      return null;
  }
}

/** Wrap a CodeMirror 5 style stream parser (`@codemirror/legacy-modes`). */
async function streamMode(
  mod: Promise<Record<string, unknown>>,
  exportName: string,
): Promise<LanguageSupport> {
  const spec = (await mod)[exportName] as StreamParser<unknown>;
  return new LanguageSupport(StreamLanguage.define(spec));
}
