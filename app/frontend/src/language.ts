//! Syntax highlighting modes (spec §8): a StreamLanguage "log" style plus
//! JSON / INI / C. Default is PlainText (no extension). Extension inference
//! picks a starting mode; the status bar can override it per document.
//!
//! Tag names are passed as strings (resolved via lezer's `tags` inside the
//! stream parser) so we don't import `@lezer/highlight` directly — it isn't a
//! declared dependency, and the default highlight style covers these classes.

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
    if (stream.sol() && stream.match(/^\[?\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}/, false)) {
      stream.skipToEnd();
      return "className"; // timestamp → distinctive class colour
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
    if (stream.sol() && stream.peek() !== undefined) {
      if (/^[#;]/.test(stream.string[stream.pos])) {
        stream.skipToEnd();
        return "comment";
      }
      if (stream.match(/^\[[^\]]*\]/, false)) {
        stream.skipToEnd();
        return "className";
      }
      if (stream.match(/^[^=\s]+(?=\s*=)/)) return "propertyName";
    }
    if (stream.eatSpace()) return null;
    if (stream.match(/#/) || stream.match(/;/)) {
      stream.skipToEnd();
      return "comment";
    }
    if (stream.match(/^[^#;\s]+/)) return "string";
    stream.next();
    return null;
  },
});

/** Highlight style mapping the log/INI tag classes to colours. */
const logHighlight = HighlightStyle.define([
  { tag: t.keyword, class: "cm-log-level" },
  { tag: t.className, class: "cm-log-ts" },
  { tag: t.number, class: "cm-log-num" },
  { tag: t.string, class: "cm-log-str" },
  { tag: t.comment, class: "cm-ini-comment" },
  { tag: t.propertyName, class: "cm-ini-key" },
]);

/**
 * Resolve a syntax mode for an extension. `""`/unknown → null = PlainText.
 * `log` returns the log language plus its highlight; JSON/C use their lezer
 * parsers (highlight comes from the language's own style); INI uses the
 * hand-written stream mode.
 */
export function modeForExtension(ext: string): LanguageSupport | null {
  switch (ext.toLowerCase()) {
    case "json":
      return json();
    case "c":
    case "h":
      return cpp();
    case "ini":
    case "cfg":
    case "conf":
      return new LanguageSupport(iniLanguage, [syntaxHighlighting(logHighlight)]);
    case "log":
    case "txt":
    default:
      // `.log` gets the log style; plain/unknown stays unstyled (PlainText).
      if (ext.toLowerCase() === "log") {
        return new LanguageSupport(logLanguage, [syntaxHighlighting(logHighlight)]);
      }
      return null;
  }
}

/** The set of languages the status-bar switcher offers. */
export const AVAILABLE_MODES = ["auto", "plain", "log", "json", "ini", "c"] as const;
export type ModeChoice = (typeof AVAILABLE_MODES)[number];

/** Map an explicit mode choice (or "" → auto) to a `modeForExtension` ext key. */
export function modeToExt(choice: ModeChoice, filename: string): string {
  if (choice === "auto") return filename; // caller passes the raw extension
  if (choice === "plain") return "";
  return choice;
}
