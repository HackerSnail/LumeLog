//! Encoding menu data + extension inference. The nine display names are pinned
//! verbatim by the plan's Global Constraints (spec §5) — `ENCODINGS` below is the
//! single source of truth the status-bar dropdown renders and the vitest
//! `encoding_menu_labels_match_spec` case asserts against, one label at a time.

export interface EncodingOption {
  /** Label sent to the shell / accepted by `EncodingLabel::from_label` (Rust). */
  label: string;
  /** Dropdown text — for the 9 fixed encodings this equals `label` exactly. */
  display: string;
}

/**
 * The 9 supported encodings, in spec order. `GB2312` is decoded as `GBK`
 * (superset) so it is not listed separately; detection falls back to `GB18030`.
 */
export const ENCODINGS: readonly EncodingOption[] = [
  { label: "UTF-8", display: "UTF-8" },
  { label: "UTF-8 BOM", display: "UTF-8 BOM" },
  { label: "UTF-16 LE", display: "UTF-16 LE" },
  { label: "UTF-16 BE", display: "UTF-16 BE" },
  { label: "GBK", display: "GBK" },
  { label: "GB18030", display: "GB18030" },
  { label: "BIG5", display: "BIG5" },
  { label: "Shift-JIS", display: "Shift-JIS" },
  { label: "ISO-8859-1", display: "ISO-8859-1" },
] as const;

/** The 9 labels, in order — handy for building a `<select>`. */
export const ENCODING_LABELS = ENCODINGS.map((e) => e.label);

/** Line-ending options surfaced next to the encoding (spec §4.4). Single source
 * of truth — the status bar renders these; `main.ts` seeds the current value
 * from the core-detected `DocInfo.eol`. */
export const EOL_OPTIONS = ["LF", "CRLF", "MIXED"] as const;

/** Lowercased extension of a path, without the dot ("" if none). */
export function extOf(path: string): string {
  const base = path.split(/[\\/]/).pop() ?? "";
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return "";
  return base.slice(dot + 1).toLowerCase();
}
