//! Tauri v2 IPC commands. Each is a thin forwarder onto `lumelog-core`; no
//! document/encoding/search logic lives here (spec §3). Heavy calls run as
//! synchronous commands for Task 8 (verification = `cargo check`); the async /
//! non-blocking indexing model is exercised by the large-file UI in Task 10.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use lumelog_can::dbc::{parse_lenient, ParseOutcome};
use lumelog_can::error::CanError;
use lumelog_can::source::{AscSource, CandumpSource, FrameSource, Line, RegexSource};
use lumelog_core::doc::{save_text, DocFile, EolStyle, OverwritePolicy, SaveOptions};
use lumelog_core::encoding::EncodingLabel;
use lumelog_core::search::{search_stream, Mode, SearchQuery};
use lumelog_core::filter::LineFilter;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::dto::*;
use crate::state::AppDocs;

/// 4 GiB u32 line-index ceiling (spec §4.1/A9). `validate_open_path` rejects at
/// the stat stage — zero I/O — with the `too_large` code before any read, matching
/// the `DocFile::open` size gate and the byte-identical `file_size > u32::MAX`
/// check inside `LineIndexBuilder::finish` (`CoreError::TooLarge`).
const SIZE_LIMIT: u64 = u32::MAX as u64;
/// Edit/read-only split (spec §4.3/A1): files up to 200 MiB mount in CodeMirror;
/// larger open in the read-only virtual list (Task 10). The shell owns this
/// threshold and returns it as `DocInfo.mode` so the front end never re-derives it.
const EDIT_THRESHOLD: u64 = 200 * 1024 * 1024;
/// Hard cap on a single `doc_chunk` request. The read-only list requests at most
/// `viewport + 2*OVERSCAN` lines (a few hundred), and edit-mode `loadAllText`
/// pages in blocks of 20 000, so no legitimate caller exceeds this. The cap is
/// the shell-side half of the spec §3 "后端永不把整个大文件交给前端" invariant:
/// a front-end sizing bug must not be able to pull a whole gigabyte across IPC
/// in one call (final-review C-1). Matches `loadAllText`'s `blockLines`.
const MAX_CHUNK_LINES: u32 = 20_000;
/// Matches flushed to the front end per `on_batch` message.
const SEARCH_BATCH: usize = 200;

/// First-paint interactivity probe (spec §9 `app_ping`).
#[tauri::command]
pub fn app_ping() -> &'static str {
    "pong"
}

/// Open a document: pre-validate (spec §9 codes), then hand the path to the core.
/// v1 indexes synchronously (fast — ~0.6 s/GB, Task 7 P3) and emits a single
/// completion tick on `on_progress`; granular 8 MiB ticks arrive with Task 10.
#[tauri::command]
pub fn doc_open(
    state: tauri::State<'_, AppDocs>,
    path: String,
    on_progress: Channel<IndexProgress>,
) -> Result<DocInfo, IpcError> {
    let p = Path::new(&path);
    lumelog_core::doc::validate_open_path(p, SIZE_LIMIT).map_err(IpcError::from_open_reject)?;
    let doc = DocFile::open(p, SIZE_LIMIT).map_err(IpcError::from_core)?;

    let size = doc.size();
    let encoding = doc.encoding().as_str().to_string();
    let eol = eol_str(doc.eol_style());
    let lines = doc.line_count() as u64;
    let mtime_millis = doc
        .mtime()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let doc_id = state.insert(doc);

    let info = DocInfo {
        doc_id,
        size,
        mtime_millis,
        encoding,
        eol,
        mode: mode_str(size),
        line_count_estimate: lines,
        total_lines: Some(lines),
    };
    let _ = on_progress.send(IndexProgress {
        doc_id,
        indexed_bytes: size,
        done: true,
    });
    Ok(info)
}

/// Decode `[line_start, line_start+line_count)` and stream it to the front end.
/// `char_map` stays core-side (the front end renders text only).
#[tauri::command]
pub fn doc_chunk(
    state: tauri::State<'_, AppDocs>,
    doc_id: u64,
    line_start: u32,
    line_count: u32,
    on_data: Channel<ChunkPayload>,
) -> Result<(), IpcError> {
    let doc_arc = state
        .get(doc_id)
        .ok_or_else(|| IpcError::new("unknown_doc", format!("no document {doc_id}")))?;
    let mut doc = doc_arc
        .lock()
        .map_err(|_| IpcError::new("io", "document mutex poisoned"))?;
    if line_count > MAX_CHUNK_LINES {
        return Err(IpcError::new(
            "invalid_argument",
            format!("line_count {line_count} exceeds chunk cap {MAX_CHUNK_LINES}"),
        ));
    }
    let chunk = doc
        .chunk_at(line_start as usize, line_count as usize)
        .map_err(IpcError::from_core)?;
    on_data
        .send(ChunkPayload {
            start_line: chunk.start_line as u64,
            text: chunk.text,
            invalid_count: chunk.invalid_count as u64,
        })
        .map_err(|e| IpcError::new("io", e.to_string()))?;
    Ok(())
}

/// Persist `text` for `doc_id`. `encoding`/`eol` are the front-end labels; a
/// `path` switches to Save-As. `force` overrides conflict / unmappable gates.
#[tauri::command]
pub fn doc_save(
    state: tauri::State<'_, AppDocs>,
    doc_id: u64,
    text: String,
    path: Option<String>,
    encoding: String,
    eol: String,
    force: bool,
) -> Result<SaveReportJson, IpcError> {
    let label = EncodingLabel::from_label(&encoding)
        .ok_or_else(|| IpcError::new("encoding_unsupported", format!("unknown encoding '{encoding}'")))?;
    let eol_style =
        parse_eol(&eol).ok_or_else(|| IpcError::new("invalid_argument", format!("unknown eol '{eol}'")))?;
    let doc_arc = state
        .get(doc_id)
        .ok_or_else(|| IpcError::new("unknown_doc", format!("no document {doc_id}")))?;
    let mut doc = doc_arc
        .lock()
        .map_err(|_| IpcError::new("io", "document mutex poisoned"))?;

    let meta = doc.disk_meta();
    let opts = SaveOptions {
        path: path.map(PathBuf::from),
        encoding: label,
        eol: eol_style,
        overwrite_policy: if force {
            OverwritePolicy::Force
        } else {
            OverwritePolicy::SafetyConfirm
        },
    };
    let report = save_text(&mut doc, &text, &opts, meta).map_err(IpcError::from_core)?;
    Ok(SaveReportJson {
        replace_count: report.replace_count as u64,
        conflict: report.conflict,
    })
}

/// Currently-open documents, for front-end restore (spec §9 `doc_list`).
#[tauri::command]
pub fn doc_list(state: tauri::State<'_, AppDocs>) -> Result<Vec<DocEntry>, IpcError> {
    let mut out = Vec::new();
    for id in state.ids() {
        if let Some(arc) = state.get(id) {
            if let Ok(doc) = arc.lock() {
                out.push(DocEntry {
                    doc_id: id,
                    path: doc.path().to_string_lossy().into_owned(),
                    encoding: doc.encoding().as_str().to_string(),
                    size: doc.size(),
                    total_lines: doc.line_count() as u64,
                });
            }
        }
    }
    Ok(out)
}

/// Stream a search over the document, returning the `request_id` used to cancel.
/// Hits are flushed in batches; the scan stops cooperatively when the cancel
/// flag flips (via `search_cancel`) between progress ticks.
#[tauri::command]
pub fn search_run(
    state: tauri::State<'_, AppDocs>,
    doc_id: u64,
    pattern: String,
    mode: String,
    case_sensitive: bool,
    on_batch: Channel<Vec<MatchJson>>,
    on_done: Channel<SearchStatsJson>,
) -> Result<u64, IpcError> {
    let (req_id, cancel) = state.begin_search();
    let doc_arc = match state.get(doc_id) {
        Some(a) => a,
        None => {
            state.end_search(req_id);
            return Err(IpcError::new("unknown_doc", format!("no document {doc_id}")));
        }
    };
    let mut doc = match doc_arc.lock() {
        Ok(d) => d,
        Err(_) => {
            state.end_search(req_id);
            return Err(IpcError::new("io", "document mutex poisoned"));
        }
    };

    let q = SearchQuery {
        pattern_text: pattern,
        mode: if mode.eq_ignore_ascii_case("regex") {
            Mode::Regex
        } else {
            Mode::Literal
        },
        encoding: doc.encoding(),
        case_sensitive,
    };

    let mut batch: Vec<MatchJson> = Vec::new();
    let result = search_stream(
        &mut doc,
        &q,
        |m| {
            batch.push(MatchJson {
                line: m.line as u64,
                byte_in_line: m.byte_in_line,
                len: m.len as u64,
                snippet: m.snippet,
            });
            if batch.len() >= SEARCH_BATCH {
                let _ = on_batch.send(std::mem::take(&mut batch));
            }
        },
        |_scanned_bytes| !cancel.load(Ordering::Relaxed),
    );
    state.end_search(req_id);
    let stats = result.map_err(IpcError::from_core)?;

    if !batch.is_empty() {
        let _ = on_batch.send(batch);
    }
    let _ = on_done.send(SearchStatsJson {
        matches: stats.matches,
        scanned_bytes: stats.scanned_bytes,
        cancelled: stats.cancelled,
    });
    Ok(req_id)
}

/// Ask an in-flight search (`search_run`) to stop.
#[tauri::command]
pub fn search_cancel(state: tauri::State<'_, AppDocs>, request_id: u64) -> Result<(), IpcError> {
    if state.cancel(request_id) {
        Ok(())
    } else {
        Err(IpcError::new(
            "unknown_request",
            format!("no in-flight search {request_id}"),
        ))
    }
}

/// Re-declare a document's encoding label (spec §8 encoding switch). Clears the
/// core chunk cache and re-reads `[line_start, line_start+line_count)` so the
/// front end re-renders text decoded under the new label. This is a genuine
/// re-decode, not a save-time relabel — the reported `encoding` is the truth.
#[tauri::command]
pub fn doc_set_encoding(
    state: tauri::State<'_, AppDocs>,
    doc_id: u64,
    encoding: String,
    line_start: u32,
    line_count: u32,
    on_data: Channel<ChunkPayload>,
) -> Result<DocInfo, IpcError> {
    let label = EncodingLabel::from_label(&encoding)
        .ok_or_else(|| IpcError::new("encoding_unsupported", format!("unknown encoding '{encoding}'")))?;
    let doc_arc = state
        .get(doc_id)
        .ok_or_else(|| IpcError::new("unknown_doc", format!("no document {doc_id}")))?;
    let mut doc = doc_arc
        .lock()
        .map_err(|_| IpcError::new("io", "document mutex poisoned"))?;
    doc.set_encoding(label);

    let chunk = doc
        .chunk_at(line_start as usize, line_count as usize)
        .map_err(IpcError::from_core)?;
    on_data
        .send(ChunkPayload {
            start_line: chunk.start_line as u64,
            text: chunk.text,
            invalid_count: chunk.invalid_count as u64,
        })
        .map_err(|e| IpcError::new("io", e.to_string()))?;

    Ok(DocInfo {
        doc_id,
        size: doc.size(),
        mtime_millis: doc
            .mtime()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        encoding: doc.encoding().as_str().to_string(),
        eol: eol_str(doc.eol_style()),
        mode: mode_str(doc.size()),
        line_count_estimate: doc.line_count() as u64,
        total_lines: Some(doc.line_count() as u64),
    })
}

/// Release a document the front end no longer shows (tab close / rejected
/// open). Drops the `DocFile` (closes the handle, frees the line index + cache)
/// so opening/closing repeatedly does not leak documents (I5).
#[tauri::command]
pub fn doc_close(state: tauri::State<'_, AppDocs>, doc_id: u64) -> Result<(), IpcError> {
    if state.remove(doc_id) {
        Ok(())
    } else {
        Err(IpcError::new(
            "unknown_doc",
            format!("no document {doc_id}"),
        ))
    }
}

/// Read + lenient-parse a DBC file. Lenient so a single bad construct does not
/// void the whole import — the good messages are kept and warnings surface (spec
/// §7.1 "跳过坏行继续"). Never panics on non-UTF-8 bytes (lossy).
pub fn import_dbc_info(path: &Path) -> Result<ParseOutcome, IpcError> {
    let bytes = std::fs::read(path).map_err(|e| IpcError::new("io", e.to_string()))?;
    let src = String::from_utf8_lossy(&bytes);
    Ok(parse_lenient(&src))
}

/// Project a parse outcome into the front-end summary: message rows (extended
/// ids re-armed with bit31 so the panel shows the true DBC id) + warnings.
fn dbc_summaries(o: &ParseOutcome) -> (Vec<DbcMsgSummary>, Vec<DbcErrorJson>) {
    let messages = o
        .database
        .messages
        .iter()
        .map(|m| DbcMsgSummary {
            id: if m.is_extended {
                m.id | 0x8000_0000
            } else {
                m.id
            },
            name: m.name.clone(),
            dlc: m.dlc,
            signal_count: m.signals.len() as u32,
        })
        .collect();
    let errors = o
        .errors
        .iter()
        .filter_map(|e| match e {
            CanError::DbcSyntax {
                line,
                context,
                kind,
            } => Some(DbcErrorJson {
                line: *line as u64,
                context: context.clone(),
                kind: kind.clone(),
            }),
            _ => None,
        })
        .collect();
    (messages, errors)
}

/// Import a DBC file (spec §9 `dbc_import`): parse lenient, cache the database,
/// return the message summary + warnings. A read failure is an `io` error.
#[tauri::command]
pub fn dbc_import(
    state: tauri::State<'_, AppDocs>,
    path: String,
) -> Result<DbcInfo, IpcError> {
    let outcome = import_dbc_info(Path::new(&path))?;
    let (messages, errors) = dbc_summaries(&outcome);
    let dbc_id = state.insert_dbc(Arc::new(outcome.database));
    Ok(DbcInfo {
        dbc_id,
        messages,
        errors,
    })
}

/// Frames flushed to the front end per `can_parse` `on_batch` message (spec §9
/// "每 ≤2ms 或 4096 条聚合" — bounded well under a storm; tuned for smooth table fill).
const PARSE_BATCH: usize = 512;
/// Cap on how many kept lines carry cu16 highlight ranges over one `filter_run`
/// (spec §3.1). Beyond it, kept lines still count and stream but with empty
/// `ranges` and `truncated=true`, so a pathological "matches almost everything"
/// scan degrades the highlight instead of ballooning memory.
const HIT_RANGE_CAP: usize = 50_000;

/// Split one `chunk_at` window into `(1-based line number, text)` pairs, dropping
/// the window's trailing terminator so the split yields `count` lines, not a
/// trailing empty one. Shared by the streaming reader and the line-order test.
fn chunk_to_lines(base: usize, text: &str) -> Vec<Line> {
    let mut body = text;
    if let Some(b) = body.strip_suffix("\r\n") {
        body = b;
    } else if let Some(b) = body.strip_suffix('\n') {
        body = b;
    }
    body.split('\n')
        .enumerate()
        .map(|(i, raw)| {
            let t = raw.strip_suffix('\r').unwrap_or(raw).to_string();
            ((base + i) as u64 + 1, t)
        })
        .collect()
}

/// Lazy, chunk-held line reader feeding a `FrameSource` (spec §3 "never hand the
/// whole file over"). Pulls one `MAX_CHUNK_LINES` window at a time from the
/// document, so peak memory is a single chunk regardless of file size — the
/// eager `Vec` of every line is gone. The `Arc<Mutex<..>>` is locked only for the
/// duration of each `chunk_at`, released before the source consumes the buffer.
pub struct DocLines {
    doc: Arc<Mutex<DocFile>>,
    total: usize,
    start: usize,
    buf: std::vec::IntoIter<Line>,
}

impl DocLines {
    /// Line count is read once up front; the body is never materialized here.
    fn new(doc: Arc<Mutex<DocFile>>) -> Self {
        let total = doc
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .line_count();
        Self {
            doc,
            total,
            start: 0,
            buf: Vec::new().into_iter(),
        }
    }

    /// Refill `buf` from the next chunk window, advancing `start`. An I/O error
    /// (or reaching the end) simply stops the stream rather than poisoning it.
    fn refill(&mut self) {
        if self.start >= self.total {
            self.buf = Vec::new().into_iter();
            return;
        }
        let count = (MAX_CHUNK_LINES as usize).min(self.total - self.start);
        let chunk = {
            let mut g = self.doc.lock().unwrap_or_else(|p| p.into_inner());
            g.chunk_at(self.start, count)
        };
        match chunk {
            Ok(c) => {
                self.start += count;
                self.buf = chunk_to_lines(c.start_line, &c.text).into_iter();
            }
            Err(_) => {
                self.start = self.total;
                self.buf = Vec::new().into_iter();
            }
        }
    }
}

impl Iterator for DocLines {
    type Item = Line;
    fn next(&mut self) -> Option<Line> {
        loop {
            if let Some(l) = self.buf.next() {
                return Some(l);
            }
            if self.start >= self.total {
                return None;
            }
            self.refill();
        }
    }
}

/// Drain a source into batched `FrameJson`, checking `cancelled` per frame and
/// flushing at most every `PARSE_BATCH` frames. Returns the source's tallies.
fn parse_frames(
    src: &mut dyn FrameSource,
    db: Option<&lumelog_can::dbc::Database>,
    emit: &mut dyn FnMut(Vec<FrameJson>),
    cancelled: &dyn Fn() -> bool,
) -> (u64, u64) {
    let mut batch: Vec<FrameJson> = Vec::new();
    while let Some(f) = src.next_frame() {
        if cancelled() {
            break;
        }
        batch.push(frame_json(&f, db));
        if batch.len() >= PARSE_BATCH {
            emit(std::mem::take(&mut batch));
        }
    }
    if !batch.is_empty() {
        emit(batch);
    }
    let s = src.stats();
    (s.decoded, s.skipped)
}

/// Stream a CAN decode of `doc_id` over `on_batch` (spec §9 `can_parse`). Returns
/// the job id used to `can_cancel` BEFORE the scan runs: the command's synchronous
/// prologue only validates the request, then hands the heavy decode to a blocking
/// task, so the front end gets the id immediately (Stop works mid-flight), the
/// main thread is never tied up parsing, and each `on_batch` flush renders as it
/// arrives rather than all at the end. Cheap validation — unknown doc/source,
/// missing pattern, and regex compilation — still happens synchronously so a bad
/// request rejects instead of streaming an empty result. Batches cap IPC traffic;
/// the cancel flag is checked per frame so a half-million-frame log stops promptly.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // IPC signature: 5 request fields + 2 channels + handles
pub fn can_parse(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
    dbc_id: Option<u64>,
    source_kind: String,
    regex: Option<String>,
    on_batch: Channel<Vec<FrameJson>>,
    on_done: Channel<CanStatsJson>,
) -> Result<u64, IpcError> {
    let (req_id, cancel) = state.begin_search();
    let fail = |e: IpcError| {
        state.end_search(req_id);
        e
    };
    let doc_arc = match state.get(doc_id) {
        Some(a) => a,
        None => return Err(fail(IpcError::new("unknown_doc", format!("no document {doc_id}")))),
    };
    // Validate the request synchronously before spawning: a bad source kind or an
    // uncompilable pattern must reject the promise, not stream zero frames.
    let pattern = match source_kind.as_str() {
        "candump" | "asc" => None,
        "regex" => match &regex {
            Some(p) => Some(p.clone()),
            None => {
                return Err(fail(IpcError::new(
                    "bad_regex",
                    "regex source requires a pattern",
                )))
            }
        },
        other => {
            return Err(fail(IpcError::new(
                "source_format",
                format!("unknown source kind '{other}'"),
            )))
        }
    };
    if let Some(p) = &pattern {
        // Compile-check via a throwaway source over an empty stream: surfaces
        // `bad_regex`/`regex_compile` now rather than inside the worker.
        if let Err(e) = RegexSource::new(std::iter::empty::<Line>(), p) {
            return Err(fail(IpcError::from_can(e)));
        }
    }
    let db_arc = dbc_id.and_then(|i| state.get_dbc(i));
    // `AppHandle` is `'static` + `Clone`, so the worker can re-fetch the managed
    // `AppDocs` to `end_search` when it finishes (a borrowed `State<'_>` cannot
    // cross into the `'static` blocking closure).
    let kind = source_kind.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let lines = DocLines::new(doc_arc);
        // The `Box<dyn FrameSource>` is created and consumed entirely on this one
        // blocking thread, so it never needs to be `Send` across the boundary.
        let mut src: Box<dyn FrameSource> = match kind.as_str() {
            "asc" => Box::new(AscSource::new(lines)),
            "regex" => Box::new(RegexSource::new(lines, &pattern.expect("validated above")).expect("regex compiled above")),
            _ => Box::new(CandumpSource::new(lines)),
        };
        let db = db_arc.as_deref();
        let mut emit = |batch: Vec<FrameJson>| {
            // A send error means the front end detached; the per-frame
            // `cancelled` check drains the remainder rather than aborting.
            let _ = on_batch.send(batch);
        };
        let cancelled = || cancel.load(Ordering::Relaxed);
        let (decoded, skipped) = parse_frames(&mut *src, db, &mut emit, &cancelled);
        let _ = on_done.send(CanStatsJson { decoded, skipped });
        app.state::<AppDocs>().end_search(req_id);
    });
    Ok(req_id)
}

/// Ask an in-flight `can_parse` to stop (shares the generic cancel registry with
/// `search_cancel`).
#[tauri::command]
pub fn can_cancel(state: tauri::State<'_, AppDocs>, request_id: u64) -> Result<(), IpcError> {
    if state.cancel(request_id) {
        Ok(())
    } else {
        Err(IpcError::new(
            "unknown_request",
            format!("no in-flight job {request_id}"),
        ))
    }
}

/// Decode a single editor selection as a frame (spec §7.4 "识别为报文"). The text
/// is fed through the chosen source; the first frame it yields is decoded. A
/// selection the source cannot parse returns `source_format`, never a panic.
#[tauri::command]
pub fn can_decode_snippet(
    state: tauri::State<'_, AppDocs>,
    text: String,
    dbc_id: Option<u64>,
    source_kind: Option<String>,
) -> Result<FrameJson, IpcError> {
    let kind = source_kind.unwrap_or_else(|| "candump".to_string());
    let lines = vec![(1u64, text)];
    let mut src: Box<dyn FrameSource> = match kind.as_str() {
        "candump" => Box::new(CandumpSource::new(lines.into_iter())),
        "asc" => Box::new(AscSource::new(lines.into_iter())),
        other => {
            return Err(IpcError::new(
                "source_format",
                format!("snippet decode supports candump/asc, not '{other}'"),
            ))
        }
    };
    match src.next_frame() {
        Some(f) => {
            let db = dbc_id.and_then(|i| state.get_dbc(i));
            Ok(frame_json(&f, db.as_deref()))
        }
        None => Err(IpcError::new(
            "source_format",
            "无法从选中文本解析报文行",
        )),
    }
}

/// Drain decoded `(1-based line, text)` pairs through `lf`, emitting kept lines
/// (include-match cu16 ranges, capped at `cap` lines) in `PARSE_BATCH` groups.
/// `cancelled` is checked per line so a huge scan stops promptly. Pure over the
/// iterator so it is unit-testable with a `Vec` (the command feeds `DocLines`).
fn scan_filter<I: Iterator<Item = Line>>(
    lines: I,
    lf: &LineFilter,
    emit: &mut dyn FnMut(Vec<LineHit>),
    cancelled: &dyn Fn() -> bool,
    cap: usize,
) -> FilterStats {
    let mut batch: Vec<LineHit> = Vec::new();
    let (mut kept, mut scanned) = (0u64, 0u64);
    let mut ranges_emitted = 0usize;
    let mut truncated = false;
    for (line1, text) in lines {
        if cancelled() {
            break;
        }
        scanned += 1;
        if let Some(ranges) = lf.keep(&text) {
            kept += 1;
            let out = if ranges.is_empty() {
                Vec::new()
            } else if ranges_emitted < cap {
                ranges_emitted += 1;
                ranges.into_iter().map(Cu16RangeJson::from).collect()
            } else {
                truncated = true;
                Vec::new()
            };
            batch.push(LineHit { line: line1 - 1, ranges: out });
            if batch.len() >= PARSE_BATCH {
                emit(std::mem::take(&mut batch));
            }
        }
    }
    if !batch.is_empty() {
        emit(batch);
    }
    FilterStats { kept, scanned, truncated }
}

/// Stream a multi-condition AND line filter (spec §6.4). Async-off-main-thread
/// like `can_parse`: the synchronous prologue validates the doc and compiles the
/// conditions (a bad pattern rejects with `invalid_pattern` before any stream),
/// then a blocking task scans the whole file while the command returns `req_id`
/// immediately — so Stop works mid-flight, batches render as they arrive, and the
/// main thread is never tied up. Only kept line numbers + cu16 ranges cross IPC;
/// line TEXT is fetched on demand via `doc_lines_at` (spec §3, never resident).
#[tauri::command]
#[allow(clippy::too_many_arguments)] // IPC: doc + conditions + case + 2 channels + handles
pub fn filter_run(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
    conditions: Vec<FilterCondition>,
    case_sensitive: bool,
    on_batch: Channel<Vec<LineHit>>,
    on_done: Channel<FilterStats>,
) -> Result<u64, IpcError> {
    let (req_id, cancel) = state.begin_search();
    let fail = |e: IpcError| {
        state.end_search(req_id);
        e
    };
    let doc_arc = match state.get(doc_id) {
        Some(a) => a,
        None => return Err(fail(IpcError::new("unknown_doc", format!("no document {doc_id}")))),
    };
    let specs: Vec<_> = conditions.iter().map(|c| c.to_core()).collect();
    if specs.is_empty() {
        // Defensive no-op: the panel guards against an empty query, but never
        // fan out into a whole-file scan that keeps every line.
        let _ = on_done.send(FilterStats {
            kept: 0,
            scanned: 0,
            truncated: false,
        });
        state.end_search(req_id);
        return Ok(req_id);
    }
    let lf = LineFilter::build(specs, case_sensitive).map_err(|e| fail(IpcError::from_core(e)))?;
    tauri::async_runtime::spawn_blocking(move || {
        let lines = DocLines::new(doc_arc);
        let mut emit = |batch: Vec<LineHit>| {
            // A send error means the front end detached; the per-line `cancelled`
            // check drains the remainder rather than aborting the task.
            let _ = on_batch.send(batch);
        };
        let cancelled = || cancel.load(Ordering::Relaxed);
        let stats = scan_filter(lines, &lf, &mut emit, &cancelled, HIT_RANGE_CAP);
        let _ = on_done.send(stats);
        app.state::<AppDocs>().end_search(req_id);
    });
    Ok(req_id)
}

/// Ask an in-flight `filter_run` to stop (shares the generic cancel registry).
#[tauri::command]
pub fn filter_cancel(state: tauri::State<'_, AppDocs>, request_id: u64) -> Result<(), IpcError> {
    if state.cancel(request_id) {
        Ok(())
    } else {
        Err(IpcError::new(
            "unknown_request",
            format!("no in-flight job {request_id}"),
        ))
    }
}

#[cfg(test)]
mod filter_run_tests {
    use super::*;
    use lumelog_core::filter::{CondKind, CondMode, CondSpec};

    fn lines(text: &str) -> Vec<Line> {
        text.lines()
            .enumerate()
            .map(|(i, l)| ((i as u64) + 1, l.to_string()))
            .collect()
    }
    fn make_filter(includes: &[(&str, bool)], excludes: &[&str], case: bool) -> LineFilter {
        let mut specs: Vec<CondSpec> = Vec::new();
        for (t, rx) in includes {
            specs.push(CondSpec {
                mode: CondMode::Include,
                kind: if *rx { CondKind::Regex } else { CondKind::Literal },
                text: (*t).to_string(),
            });
        }
        for t in excludes {
            specs.push(CondSpec {
                mode: CondMode::Exclude,
                kind: CondKind::Literal,
                text: (*t).to_string(),
            });
        }
        LineFilter::build(specs, case).unwrap()
    }

    #[test]
    fn scan_filter_and_keeps_correct_lines() {
        let lf = make_filter(&[("ERROR", false)], &["debug"], true);
        let mut got: Vec<LineHit> = Vec::new();
        let stats = scan_filter(
            lines("ERROR a\ndebug b\nERROR c").into_iter(),
            &lf,
            &mut |b| got.extend(b),
            &|| false,
            usize::MAX,
        );
        assert_eq!(stats.kept, 2);
        assert_eq!(stats.scanned, 3);
        assert_eq!(
            got.iter().map(|h| h.line).collect::<Vec<_>>(),
            vec![0, 2],
            "0-based, debug-excluded"
        );
    }

    #[test]
    fn scan_filter_emits_cu16_ranges() {
        let lf = make_filter(&[("a", false)], &[], true);
        let mut got: Vec<LineHit> = Vec::new();
        scan_filter(
            lines("ba\n").into_iter(),
            &lf,
            &mut |b| got.extend(b),
            &|| false,
            usize::MAX,
        );
        assert_eq!(got[0].ranges[0].cu16_start, 1);
        assert_eq!(got[0].ranges[0].cu16_len, 1);
    }

    #[test]
    fn scan_filter_cancel_stops_early() {
        let lf = make_filter(&[("ERROR", false)], &[], true);
        let mut text = String::new();
        for _ in 0..10 {
            text.push_str("ERROR x\n");
        }
        let seen = std::cell::Cell::new(0u32);
        let mut got: Vec<LineHit> = Vec::new();
        let stats = scan_filter(
            lines(&text).into_iter(),
            &lf,
            &mut |b| got.extend(b),
            &|| {
                let c = seen.get();
                seen.set(c + 1);
                c >= 3
            },
            usize::MAX,
        );
        assert!(stats.kept < 10, "cancel stops early (kept {})", stats.kept);
        assert!(!got.is_empty(), "some hits emitted before cancel");
    }

    #[test]
    fn scan_filter_truncates_over_cap() {
        let lf = make_filter(&[("x", false)], &[], true);
        let mut got: Vec<LineHit> = Vec::new();
        let stats = scan_filter(
            lines("x\nx\nx\nx\n").into_iter(),
            &lf,
            &mut |b| got.extend(b),
            &|| false,
            2,
        );
        assert_eq!(stats.kept, 4);
        assert!(stats.truncated, "cap 2 over 4 range-bearing hits truncates");
        assert_eq!(got[0].ranges.len(), 1);
        assert_eq!(got[1].ranges.len(), 1);
        assert!(got[2].ranges.is_empty() && got[3].ranges.is_empty(), "later hits drop ranges");
    }
}

fn eol_str(e: EolStyle) -> String {
    match e {
        EolStyle::Lf => "lf".to_string(),
        EolStyle::Crlf => "crlf".to_string(),
        EolStyle::Mixed => "mixed".to_string(),
    }
}

fn mode_str(size: u64) -> String {
    if size <= EDIT_THRESHOLD {
        "edit".to_string()
    } else {
        "readonly".to_string()
    }
}

fn parse_eol(s: &str) -> Option<EolStyle> {
    match s.to_ascii_lowercase().as_str() {
        "lf" => Some(EolStyle::Lf),
        "crlf" => Some(EolStyle::Crlf),
        "mixed" => Some(EolStyle::Mixed),
        _ => None,
    }
}

#[cfg(test)]
mod can_import_tests {
    use super::*;

    fn sample() -> std::path::PathBuf {
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/lumelog-can/tests/data/sample.dbc"
        ))
        .to_path_buf()
    }

    #[test]
    fn dbc_import_reads_sample_dbc() {
        let o = import_dbc_info(&sample()).unwrap();
        let (messages, errors) = dbc_summaries(&o);
        // exactly one deliberately broken SG_ line surfaces as a lenient warning
        assert_eq!(errors.len(), 1, "one bad SG_ -> one warning");
        assert_eq!(errors[0].kind, "bad SG_");
        assert!(errors[0].context.contains("BrokenSignal"));
        // EngineData keeps its 2 good signals; the broken one is dropped
        let eng = messages
            .iter()
            .find(|m| m.name == "EngineData")
            .expect("EngineData present");
        assert_eq!(eng.signal_count, 2, "broken SG_ dropped, 2 kept");
        // ExtFrame's masked id is re-armed with bit31 (raw DBC id 2147484928)
        let ext = messages
            .iter()
            .find(|m| m.name == "ExtFrame")
            .expect("ExtFrame present");
        assert_eq!(ext.id, 2147484928, "extended id reconstructed");
        let mux = messages.iter().find(|m| m.name == "MuxedMsg").unwrap();
        assert_eq!(mux.signal_count, 3);
    }

    #[test]
    fn dbc_import_missing_file_is_io_error() {
        let err = import_dbc_info(std::path::Path::new("Z:/no-such-file.dbc")).unwrap_err();
        assert_eq!(err.code, "io");
    }
}

#[cfg(test)]
mod can_parse_tests {
    use super::*;
    use lumelog_can::frame::Frame;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn doc_of(content: &str) -> (NamedTempFile, DocFile) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        let doc = DocFile::open(f.path(), SIZE_LIMIT).unwrap();
        (f, doc)
    }

    #[test]
    fn doc_lines_preserves_line_order() {
        let (_keep, doc) = doc_of("alpha\r\nbravo\r\ncharlie\r\n");
        let lines: Vec<Line> = DocLines::new(Arc::new(Mutex::new(doc))).collect();
        assert_eq!(lines.len(), 3, "three \r\n-terminated lines");
        assert_eq!(lines[0], (1, "alpha".to_string()));
        assert_eq!(lines[1], (2, "bravo".to_string()));
        assert_eq!(lines[2], (3, "charlie".to_string()));
    }

    #[test]
    fn parse_frames_decodes_candump_and_counts_skips() {
        let content = " (1.5) can0 100#0000960012340000\nnot-a-frame\ncan0 100#DEADBEEF\n";
        let (_k, doc) = doc_of(content);
        let lines: Vec<Line> = DocLines::new(Arc::new(Mutex::new(doc))).collect();
        let mut src = CandumpSource::new(lines.into_iter());
        let mut got: Vec<FrameJson> = Vec::new();
        let (decoded, skipped) = parse_frames(&mut src, None, &mut |b| got.extend(b), &|| false);
        assert_eq!(decoded, 2, "two frame lines");
        assert_eq!(skipped, 1, "the garbage line is skipped");
        assert!(got.iter().all(|f| f.can_id == 0x100));
        assert_eq!(got[0].timestamp_ms, Some(1500.0), "1.5 s -> 1500 ms");
        assert_eq!(got[0].line_number, 1, "1-based source line");
    }

    #[test]
    fn parse_frames_stops_on_cancel() {
        let mut content = String::new();
        for i in 0..20 {
            content.push_str(&format!("can0 100#{:02X}DEADBEEF\n", i));
        }
        let (_k, doc) = doc_of(&content);
        let lines: Vec<Line> = DocLines::new(Arc::new(Mutex::new(doc))).collect();
        let mut src = CandumpSource::new(lines.into_iter());
        let seen = std::cell::Cell::new(0u32);
        let mut got: Vec<FrameJson> = Vec::new();
        parse_frames(
            &mut src,
            None,
            &mut |b| got.extend(b),
            &|| {
                let c = seen.get();
                seen.set(c + 1);
                c >= 3
            },
        );
        assert!(got.len() < 20, "cancel stops early (got {})", got.len());
        assert!(!got.is_empty(), "at least one frame emitted before cancel");
    }

    #[test]
    fn frame_json_reconstructs_data_hex_for_full_payload() {
        let mut data = [0u8; 64];
        for (i, b) in data.iter_mut().enumerate() {
            *b = i as u8;
        }
        let f = Frame {
            line_number: 1,
            timestamp: None,
            channel: 0,
            can_id: 0x7AA,
            dlc: 64,
            data,
            flags: Default::default(),
        };
        let j = frame_json(&f, None);
        assert_eq!(j.dlc, 64);
        assert_eq!(j.data_hex.len(), 128, "64 bytes -> 128 hex chars, no panic");
        assert!(j.data_hex.starts_with("000102"));
    }
}
