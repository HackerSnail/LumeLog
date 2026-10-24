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
  let payload: ChunkPayload | null = null;
  ch.onmessage = (c) => {
    payload = c;
  };
  await docChunk(docId, start, count, ch);
  if (!payload) return { first: start, lines: [], invalidCount: 0 };
  const p = payload as ChunkPayload;
  // The core joins logical lines with `\n`; a trailing newline would add a phantom
  // empty line, so drop only a single trailing one before splitting.
  const body = p.text.endsWith("\n") ? p.text.slice(0, -1) : p.text;
  const lines = count === 0 ? [] : body.split("\n");
  return { first: p.startLine, lines, invalidCount: p.invalidCount };
}
