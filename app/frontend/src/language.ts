//! Syntax highlighting modes (spec §8): a StreamLanguage "log" style plus
//! JSON / INI / C. Default is PlainText (no extension). Extension inference
//! picks a starting mode; the status bar can override it per document.
//!
//! Highlight colors are inline (`color`/`fontWeight`), not CSS classes, so the
//! log/INI styles work without a stylesheet — CodeMirror's `HighlightStyle`
//! emits its own managed CSS. JSON/C get colours from `defaultHighlightStyle`
//! (mounted alongside the language in `editor.ts`), which maps the tags their
//! lezer parsers emit.

import {
  HighlightStyle,
  StreamLanguage,
  syntaxHighlighting,
} from "@codemirror/language";
import { json } from "@codemirror/lang-json";
import { cpp } from "@codemirror/lang-cpp";
import { LanguageSupport } from "@codemirror/language";
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

/**
 * Resolve a syntax mode for an extension. `""`/unknown → null = PlainText.
 * `log`/`ini` return their stream language + the inline highlight; JSON/C use
 * their lezer parsers (colour supplied by `defaultHighlightStyle` in editor.ts).
 */
export function modeForExtension(ext: string): LanguageSupport | null {
  const e = ext.toLowerCase();
  switch (e) {
    case "json":
      return json();
    case "c":
    case "h":
    case "cpp":
    case "cc":
      return cpp();
    case "ini":
    case "cfg":
    case "conf":
      return new LanguageSupport(iniLanguage, [syntaxHighlighting(streamHighlight)]);
    case "log":
      return new LanguageSupport(logLanguage, [syntaxHighlighting(streamHighlight)]);
    default:
      return null; // PlainText
  }
}

/** The set of languages the status-bar switcher offers. */
export const AVAILABLE_MODES = ["auto", "plain", "log", "json", "ini", "c"] as const;
export type ModeChoice = (typeof AVAILABLE_MODES)[number];
