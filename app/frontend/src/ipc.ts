//! IPC boundary types + helpers (spec §9). DTO field names are camelCase to
//! match the Rust serde `rename_all = "camelCase"` structs in `app/src-tauri/
//! src/dto.rs`. The error-code table is the one pure module the front end owns:
//! the shell returns stable lowercase-snake codes, we map them to Chinese copy.

import { invoke, Channel } from "@tauri-apps/api/core";

/** Summary returned by `doc_open` (Rust `DocInfo`). */
export interface DocInfo {
  docId: number;
  size: number;
  mtimeMillis: number;
  encoding: string;
  /** Dominant EOL the core detected on open — save back with this (spec §4.4). */
  eol: string;
  /** Which view to mount: `"edit"` (≤200MB) or `"readonly"` (virtual list). */
  mode: string;
  lineCountEstimate: number;
  totalLines: number | null;
}

/** One decoded block streamed over a channel (Rust `ChunkPayload`). */
export interface ChunkPayload {
  startLine: number;
  text: string;
  invalidCount: number;
}

/** Background index progress tick (Rust `IndexProgress`). */
export interface IndexProgress {
  docId: number;
  indexedBytes: number;
  done: boolean;
}

/** A single search hit (Rust `MatchJson`). */
export interface MatchJson {
  line: number;
  byteInLine: number;
  len: number;
  snippet: string;
}

/** Terminal search tally (Rust `SearchStatsJson`). */
export interface SearchStatsJson {
  matches: number;
  scannedBytes: number;
  cancelled: boolean;
}

/** Outcome of `doc_save` (Rust `SaveReportJson`). */
export interface SaveReportJson {
  replaceCount: number;
  conflict: boolean;
}

/** A currently-open document row (Rust `DocEntry`). */
export interface DocEntry {
  docId: number;
  path: string;
  encoding: string;
  size: number;
  totalLines: number;
}

/** A decoded signal within a frame (Rust `SignalValueJson`). */
export interface SignalValueJson {
  name: string;
  raw: number;
  physical: number;
  enumText?: string;
  unit: string;
}

/** One frame + optional DBC decode, streamed by `can_parse` (Rust `FrameJson`). */
export interface FrameJson {
  lineNumber: number;
  timestampMs?: number;
  channel: number;
  canId: number;
  canIdHex: string;
  dlc: number;
  dataHex: string;
  isRemote: boolean;
  isFd: boolean;
  errorFrame: boolean;
  msgName?: string;
  signals: SignalValueJson[];
}

/** A message row of the `dbc_import` summary (Rust `DbcMsgSummary`). */
export interface DbcMsgSummary {
  id: number;
  name: string;
  dlc: number;
  signalCount: number;
}

/** A lenient-parse warning row (Rust `DbcErrorJson`). */
export interface DbcErrorJson {
  line: number;
  context: string;
  kind: string;
}

/** Outcome of `dbc_import` (Rust `DbcInfo`). */
export interface DbcInfo {
  dbcId: number;
  messages: DbcMsgSummary[];
  errors: DbcErrorJson[];
}

/** Terminal `can_parse` tally (Rust `CanStatsJson`). */
export interface CanStatsJson {
  decoded: number;
  skipped: number;
}

// ---- M3 filter DTOs (spec §6.4 / §9). camelCase to match dto.rs -------------

/** One query-panel condition (`filter_run`). */
export interface FilterCondition {
  mode: "include" | "exclude";
  kind: "literal" | "regex";
  text: string;
}

/** A highlight span in UTF-16 code units within a line (JS `String.slice`). */
export interface Cu16Range {
  cu16Start: number;
  cu16Len: number;
}

/** One kept line + its include-match highlight ranges, batched by `filterRun`. */
export interface LineHit {
  line: number;
  ranges: Cu16Range[];
}

/** Terminal `filterRun` tally. `truncated` = range output was capped. */
export interface FilterStats {
  kept: number;
  scanned: number;
  truncated: boolean;
}

/** One requested line's decoded text (Rust `IndexedText`, from `doc_lines_at`). */
export interface IndexedText {
  index: number;
  text: string;
}

/** Structured IPC error shape as it crosses the boundary (Rust `IpcError`). */
export interface IpcErrorPayload {
  code: string;
  message: string;
  detail?: string;
}

/**
 * A recognized IPC error code. Mirrors the codes emitted by the Rust shell:
 * the 8 engine codes pinned by plan Task 9 Step 3, plus the pre-open and
 * dispatch-level codes the shell can also surface.
 */
export type IpcErrorCode =
  | "io"
  | "too_large"
  | "conflict"
  | "unmappable"
  | "locked"
  | "line_out_of_range"
  | "encoding_unsupported"
  | "invalid_pattern"
  | "not_found"
  | "is_directory"
  | "permission_denied"
  | "unknown_doc"
  | "unknown_request"
  | "invalid_argument"
  | "dbc_syntax"
  | "bad_regex"
  | "regex_compile"
  | "source_format"
  | "unknown";

/**
 * Chinese copy for each IPC error code (Global Constraints: user-facing text is
 * Chinese). Keys cover every code the shell emits; `${code}_zh` in the table
 * below is the exhaustive set the vitest `ipc_error_code_maps_to_zh_text`
 * case asserts against. `%s` is substituted with `payload.detail` when present.
 */
const IPC_ERROR_ZH: Record<IpcErrorCode, string> = {
  io: "读写文件时发生错误",
  too_large: "文件超过 4GB 上限，无法打开%s",
  conflict: "文件已被外部修改，保存会与磁盘改动冲突%s",
  unmappable: "当前编码无法表示文本中的某些字符（%s 处）",
  locked: "文件正被其他程序占用，无法写入",
  line_out_of_range: "请求的行号超出范围",
  encoding_unsupported: "不支持的编码格式",
  invalid_pattern: "搜索模式无效（空或正则语法错误）",
  not_found: "文件不存在",
  is_directory: "所选路径是目录，请选择文件",
  permission_denied: "没有权限访问该文件",
  unknown_doc: "文档已关闭或不存在",
  unknown_request: "该搜索请求已结束或不存在",
  invalid_argument: "参数无效",
  dbc_syntax: "DBC 解析出错%s",
  bad_regex: "解析正则无效（需含命名组 id/data）%s",
  regex_compile: "正则编译失败%s",
  source_format: "日志源格式无法识别%s",
  unknown: "调用失败%s",
};

/** All codes the shell can surface — exported so the test can pin full coverage. */
export const IPC_ERROR_CODES = Object.keys(IPC_ERROR_ZH) as IpcErrorCode[];

/** Map an IPC error code (+ optional detail) to a Chinese user-facing message. */
export function ipcErrorZh(code: string, detail?: string): string {
  const template = (IPC_ERROR_ZH as Record<string, string | undefined>)[code];
  const hasDetail = detail !== undefined && detail !== "";
  const suffix = hasDetail ? `：${detail}` : "";
  if (template === undefined) return `发生未知错误${suffix}`;
  if (template.includes("%s")) return template.replace("%s", suffix);
  return template;
}

/** Normalize whatever `invoke` rejected with into an `IpcErrorPayload`. */
export function toIpcError(err: unknown): IpcErrorPayload {
  if (
    typeof err === "object" &&
    err !== null &&
    "code" in err &&
    "message" in err
  ) {
    const e = err as IpcErrorPayload;
    return { code: e.code, message: e.message, detail: e.detail };
  }
  return { code: "unknown", message: String(err) };
}

/** A rejected IPC call, carrying the structured payload for the UI. */
export class IpcError extends Error {
  readonly payload: IpcErrorPayload;
  readonly zh: string;
  constructor(payload: IpcErrorPayload) {
    super(payload.message);
    this.name = "IpcError";
    this.payload = payload;
    this.zh = ipcErrorZh(payload.code, payload.detail);
  }
}

/**
 * Thin `invoke` wrapper that rethrows any rejection as a typed `IpcError`, so
 * callers get `.zh` for display and `.payload.code` for branching (e.g. the
 * save conflict / unmappable three-choice dialogs).
 */
export async function call<T>(
  cmd: string,
  args: Record<string, unknown> = {},
): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    throw new IpcError(toIpcError(err));
  }
}

// ---- Command wrappers (spec §9 names; channel params are the last arg) --------

export function appPing(): Promise<string> {
  return call<string>("app_ping");
}

export function docOpen(
  path: string,
  onProgress: (p: IndexProgress) => void,
): Promise<DocInfo> {
  const ch = new Channel<IndexProgress>();
  ch.onmessage = onProgress;
  return call<DocInfo>("doc_open", { path, onProgress: ch });
}

export function docChunk(
  docId: number,
  lineStart: number,
  lineCount: number,
  onData: Channel<ChunkPayload>,
): Promise<void> {
  return call<void>("doc_chunk", { docId, lineStart, lineCount, onData });
}

export function docSave(args: {
  docId: number;
  text: string;
  path?: string | null;
  encoding: string;
  eol: string;
  force: boolean;
}): Promise<SaveReportJson> {
  return call<SaveReportJson>("doc_save", args);
}

export function docList(): Promise<DocEntry[]> {
  return call<DocEntry[]>("doc_list");
}

export function searchRun(args: {
  docId: number;
  pattern: string;
  mode: string;
  caseSensitive: boolean;
  onBatch: (m: MatchJson[]) => void;
  onDone: (s: SearchStatsJson) => void;
}): Promise<number> {
  const batchCh = new Channel<MatchJson[]>();
  batchCh.onmessage = args.onBatch;
  const doneCh = new Channel<SearchStatsJson>();
  doneCh.onmessage = args.onDone;
  return call<number>("search_run", {
    docId: args.docId,
    pattern: args.pattern,
    mode: args.mode,
    caseSensitive: args.caseSensitive,
    onBatch: batchCh,
    onDone: doneCh,
  });
}

export function searchCancel(requestId: number): Promise<void> {
  return call<void>("search_cancel", { requestId });
}

/** Concatenate every `doc_chunk` block for `[0, totalLines)` — edit-mode full load. */
export async function loadAllText(
  docId: number,
  totalLines: number,
  blockLines = 20000,
): Promise<{ text: string; invalidCount: number }> {
  let text = "";
  let invalid = 0;
  for (let start = 0; start < totalLines; start += blockLines) {
    const count = Math.min(blockLines, totalLines - start);
    const ch = new Channel<ChunkPayload>();
    ch.onmessage = (c) => {
      text += c.text;
      invalid += c.invalidCount;
    };
    // No manual release: the JS `Channel` unregisters its callback as soon as
    // the Rust command returns and drops its `Channel` (the `end` marker).
    await docChunk(docId, start, count, ch);
  }
  return { text, invalidCount: invalid };
}

/**
 * Re-declare a document's encoding and re-read `[0, totalLines)` in one shell
 * round-trip. The Rust `doc_set_encoding` clears the core cache, re-reads the
 * first block (delivered over `onData`, its `startLine` is 0), and returns the
 * updated `DocInfo` whose `encoding` is the now-authoritative label (spec §8).
 */
export function docSetEncoding(
  docId: number,
  encoding: string,
  totalLines: number,
  onData: (c: ChunkPayload) => void,
): Promise<DocInfo> {
  const ch = new Channel<ChunkPayload>();
  ch.onmessage = onData;
  // The command's return drops the Rust `Channel`, which fires the `end` marker
  // and unregisters this JS callback — nothing to release manually.
  return call<DocInfo>("doc_set_encoding", {
    docId,
    encoding,
    lineStart: 0,
    lineCount: Math.max(1, Math.min(20000, totalLines)),
    onData: ch,
  });
}

/** Release the shell-side document (closes handle + frees index/cache, I5). */
export function docClose(docId: number): Promise<void> {
  return call<void>("doc_close", { docId });
}

/**
 * Fetch one contiguous window `[start, start+count)` of decoded lines for the
 * read-only virtual list (spec §4.3). Unlike `loadAllText` this is bounded to
 * the requested window — the whole file is never resident. `first` is the
 * absolute line index of `lines[0]` (the shell echoes `startLine`).
 */
export async function loadWindow(
  docId: number,
  start: number,
  count: number,
): Promise<{ first: number; lines: string[]; invalidCount: number }> {
  const ch = new Channel<ChunkPayload>();
  let payload: ChunkPayload | undefined;
  ch.onmessage = (c) => {
    payload = c;
  };
  await docChunk(docId, start, count, ch);
  const p = payload;
  if (!p) return { first: start, lines: [], invalidCount: 0 };
  // The core keeps each line's raw terminator in the chunk text, so a CRLF
  // file arrives with a `\r` before every `\n`. Split on `\r\n`/`\n` only — a
  // bare trailing `\r` (no LF) is line *content* in a mixed-EOL log and must
  // survive. `pre`-rendered rows and copy-paste must not carry stray CRs
  // (CodeMirror normalises internally, a raw list does not).
  const body = p.text.endsWith("\r\n")
    ? p.text.slice(0, -2)
    : p.text.endsWith("\n")
      ? p.text.slice(0, -1)
      : p.text;
  const lines = count === 0 ? [] : body.split(/\r\n|\n/);
  return { first: p.startLine, lines, invalidCount: p.invalidCount };
}

// ---- CAN commands (spec §7.4 / §9) -----------------------------------------

/** Import a DBC file; returns the cached `dbcId` + message summary + warnings. */
export function dbcImport(path: string): Promise<DbcInfo> {
  return call<DbcInfo>("dbc_import", { path });
}

/**
 * Stream a CAN decode of `docId`. Frames arrive in batches on `onBatch`; the
 * tally arrives once on `onDone`. Resolves with the job id for `canCancel`.
 * Mirrors `searchRun`'s dual-channel shape.
 */
export function canParse(args: {
  docId: number;
  dbcId?: number | null;
  sourceKind: string;
  regex?: string | null;
  onBatch: (f: FrameJson[]) => void;
  onDone: (s: CanStatsJson) => void;
}): Promise<number> {
  const batchCh = new Channel<FrameJson[]>();
  batchCh.onmessage = args.onBatch;
  const doneCh = new Channel<CanStatsJson>();
  doneCh.onmessage = args.onDone;
  return call<number>("can_parse", {
    docId: args.docId,
    dbcId: args.dbcId ?? null,
    sourceKind: args.sourceKind,
    regex: args.regex ?? null,
    onBatch: batchCh,
    onDone: doneCh,
  });
}

/** Ask an in-flight `canParse` to stop. */
export function canCancel(requestId: number): Promise<void> {
  return call<void>("can_cancel", { requestId });
}

/** Decode a single editor selection as one frame ("识别为报文"). */
export function canDecodeSnippet(args: {
  text: string;
  dbcId?: number | null;
  sourceKind?: string | null;
}): Promise<FrameJson> {
  return call<FrameJson>("can_decode_snippet", {
    text: args.text,
    dbcId: args.dbcId ?? null,
    sourceKind: args.sourceKind ?? null,
  });
}

/** Timeline timestamp format (M3 spec §3.1). */
export type TimeFormat = "iso" | "epoch_s" | "epoch_ms";

/** A timeline constraint: a regex with a named group `ts` + a format + an
 *  optional `[from,to]` window (strings parsed by `format`). */
export interface TimeSpec {
  extractPattern: string;
  format: TimeFormat;
  from?: string;
  to?: string;
  keepWhenNoTs: boolean;
}

/** Terminal `exportLines` tally (Rust `ExportStatsJson`). */
export interface ExportStats {
  written: number;
  scanned: number;
  cancelled: boolean;
  errored: boolean;
}

/**
 * Stream a multi-condition AND line filter (spec §3.1). Kept lines arrive in
 * batches on `onBatch` (line number + cu16 highlight ranges only — never line
 * text); the tally arrives once on `onDone`. Resolves with the job id for
 * `filterCancel`. Optional `time` (timeline axis) / `lineFrom`/`lineTo`
 * (line-range degrade) AND-combine with `conditions`. Mirrors `canParse`.
 */
export function filterRun(args: {
  docId: number;
  conditions: FilterCondition[];
  caseSensitive: boolean;
  time?: TimeSpec | null;
  lineFrom?: number | null;
  lineTo?: number | null;
  onBatch: (h: LineHit[]) => void;
  onDone: (s: FilterStats) => void;
}): Promise<number> {
  const batchCh = new Channel<LineHit[]>();
  batchCh.onmessage = args.onBatch;
  const doneCh = new Channel<FilterStats>();
  doneCh.onmessage = args.onDone;
  return call<number>("filter_run", {
    docId: args.docId,
    conditions: args.conditions,
    caseSensitive: args.caseSensitive,
    time: args.time ?? null,
    lineFrom: args.lineFrom ?? null,
    lineTo: args.lineTo ?? null,
    onBatch: batchCh,
    onDone: doneCh,
  });
}

/** Ask an in-flight `filterRun` to stop. */
export function filterCancel(requestId: number): Promise<void> {
  return call<void>("filter_cancel", { requestId });
}

/**
 * Fetch decoded text for a sparse set of original line indices (spec §3.2) —
 * the filtered read-only list's on-demand window loader. Text flows back over
 * a channel like `docChunk`.
 */
export function docLinesAt(args: {
  docId: number;
  indices: number[];
  onData: (p: { lines: IndexedText[]; invalidCount: number }) => void;
}): Promise<void> {
  const ch = new Channel<{ lines: IndexedText[]; invalidCount: number }>();
  ch.onmessage = args.onData;
  return call<void>("doc_lines_at", { docId: args.docId, indices: args.indices, onData: ch });
}

/**
 * Export the lines the current filter/timeline/line-range selects, as raw source
 * bytes (exact encoding + line endings), to `targetPath` (spec §7.5). Progress
 * streams on `onProgress`, final tally on `onDone`; resolves with the job id for
 * `exportCancel`.
 */
export function exportLines(args: {
  docId: number;
  conditions: FilterCondition[];
  caseSensitive: boolean;
  time?: TimeSpec | null;
  lineFrom?: number | null;
  lineTo?: number | null;
  targetPath: string;
  onProgress: (p: { written: number }) => void;
  onDone: (s: ExportStats) => void;
}): Promise<number> {
  const progCh = new Channel<{ written: number }>();
  progCh.onmessage = args.onProgress;
  const doneCh = new Channel<ExportStats>();
  doneCh.onmessage = args.onDone;
  return call<number>("export_lines", {
    docId: args.docId,
    conditions: args.conditions,
    caseSensitive: args.caseSensitive,
    time: args.time ?? null,
    lineFrom: args.lineFrom ?? null,
    lineTo: args.lineTo ?? null,
    targetPath: args.targetPath,
    onProgress: progCh,
    onDone: doneCh,
  });
}

/** Ask an in-flight `exportLines` to stop. */
export function exportCancel(requestId: number): Promise<void> {
  return call<void>("export_cancel", { requestId });
}
