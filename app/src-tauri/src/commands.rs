//! Tauri v2 IPC commands. Each is a thin forwarder onto `lumelog-core`; no
//! document/encoding/search logic lives here (spec §3). Heavy calls run as
//! synchronous commands for Task 8 (verification = `cargo check`); the async /
//! non-blocking indexing model is exercised by the large-file UI in Task 10.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::io::{BufWriter, Write};
use std::time::UNIX_EPOCH;

use lumelog_can::dbc::{parse_lenient, ParseOutcome};
use lumelog_can::error::CanError;
use lumelog_can::source::{AscSource, CandumpSource, FrameSource, Line, RegexSource};
use lumelog_core::doc::{save_text, DocFile, EolStyle, OverwritePolicy, SaveOptions};
use lumelog_core::encoding::EncodingLabel;
use lumelog_core::search::{search_stream, Mode, SearchQuery};
use lumelog_core::filter::LineFilter;
use lumelog_core::export;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::dto::*;
use crate::can_export::export_can;
use crate::watch::{AppWatch, ExternalEvent, WatchStatus};
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
    app: AppHandle,
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
    // Watch the document's directory so external modifications reach the UI.
    app.state::<AppWatch>().watch_path(Path::new(&path));

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

/// Reopen `doc_id`'s file from disk (external change): refresh the `DocFile` in
/// place so the id/tab stay stable. Testable without `State`.
fn reload_doc(state: &AppDocs, doc_id: u64) -> Result<DocInfo, IpcError> {
    let arc = state
        .get(doc_id)
        .ok_or_else(|| IpcError::new("unknown_doc", format!("no document {doc_id}")))?;
    let path = {
        let d = arc.lock().unwrap_or_else(|p| p.into_inner());
        d.path().to_path_buf()
    };
    lumelog_core::doc::validate_open_path(&path, SIZE_LIMIT)
        .map_err(IpcError::from_open_reject)?;
    let doc = DocFile::open(&path, SIZE_LIMIT).map_err(IpcError::from_core)?;
    let size = doc.size();
    let encoding = doc.encoding().as_str().to_string();
    let eol = eol_str(doc.eol_style());
    let lines = doc.line_count() as u64;
    let mtime_millis = doc
        .mtime()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if !state.replace(doc_id, doc) {
        return Err(IpcError::new("unknown_doc", format!("no document {doc_id}")));
    }
    Ok(DocInfo {
        doc_id,
        size,
        mtime_millis,
        encoding,
        eol,
        mode: mode_str(size),
        line_count_estimate: lines,
        total_lines: Some(lines),
    })
}

/// Reload a document after an external change (spec §4.4). Rebuilds the line
/// index and returns the fresh `DocInfo` (mode may have changed).
#[tauri::command]
pub fn doc_reload(
    state: tauri::State<'_, AppDocs>,
    doc_id: u64,
    on_progress: Channel<IndexProgress>,
) -> Result<DocInfo, IpcError> {
    let info = reload_doc(&state, doc_id)?;
    let _ = on_progress.send(IndexProgress {
        doc_id,
        indexed_bytes: info.size,
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
#[allow(clippy::too_many_arguments)] // IPC: doc + app + text + path/encoding/eol/force
pub fn doc_save(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
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
    // Do all doc work under the lock, then drop it before touching the watcher:
    // `notify`'s watch/unwatch waits for its event thread, which may be inside
    // `snapshot_for_path` waiting for this very doc lock.
    let (report, retarget) = {
        let mut doc = doc_arc
            .lock()
            .map_err(|_| IpcError::new("io", "document mutex poisoned"))?;
        let meta = doc.disk_meta();
        let old_path = doc.path().to_path_buf();
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
        // Save-As: rebind the doc to the file the tab now edits so path, index,
        // cache, disk snapshot and the conflict baseline all follow it.
        let mut retarget = None;
        if let Some(p) = &opts.path {
            if p != &old_path {
                let fresh = DocFile::open(p, SIZE_LIMIT).map_err(IpcError::from_core)?;
                *doc = fresh;
                retarget = Some((old_path, p.clone()));
            }
        }
        (report, retarget)
    };
    if let Some((old, new)) = retarget {
        app.state::<AppWatch>().retarget(&old, &new);
    }
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
pub fn doc_close(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
) -> Result<(), IpcError> {
    if let Some(arc) = state.get(doc_id) {
        let path = {
            let d = arc.lock().unwrap_or_else(|p| p.into_inner());
            d.path().to_path_buf()
        };
        app.state::<AppWatch>().unwatch_path(&path);
    }
    if state.remove(doc_id) {
        Ok(())
    } else {
        Err(IpcError::new(
            "unknown_doc",
            format!("no document {doc_id}"),
        ))
    }
}

/// Subscribe to external-change events (spec §4.4). Returns the degradation
/// state: a non-`None` reason means only save-time checks remain.
#[tauri::command]
pub fn watch_subscribe(
    app: AppHandle,
    on_event: Channel<ExternalEvent>,
) -> Result<WatchStatus, IpcError> {
    Ok(app.state::<AppWatch>().start(&app, on_event))
}

/// Current watcher health (degradation may appear after the initial subscribe).
#[tauri::command]
pub fn watch_status(app: AppHandle) -> Result<WatchStatus, IpcError> {
    Ok(app.state::<AppWatch>().status())
}

// ---- keymap overrides (spec §8/§10, M3 C) -----------------------------------

/// Read `config.toml`: a missing file is empty, but a real I/O error surfaces
/// so callers never overwrite a config they could not read.
fn read_config_text(app: &AppHandle) -> Result<String, IpcError> {
    let path = config_path(app)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(IpcError::new("io", format!("read config: {e}"))),
    }
}

fn write_config_text(app: &AppHandle, text: &str) -> Result<(), IpcError> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| IpcError::new("io", format!("create config dir: {e}")))?;
    }
    std::fs::write(&path, text).map_err(|e| IpcError::new("io", format!("write config: {e}")))
}

/// Current `[keymap]` overrides (absent keys mean defaults).
#[tauri::command]
pub fn keymap_get(app: AppHandle) -> Result<std::collections::BTreeMap<String, String>, IpcError> {
    Ok(crate::keymap::read_keymap(&read_config_text(&app)?))
}

/// Set (`Some(accel)`, `""` = unbound) or clear (`None` = default) one binding.
#[tauri::command]
pub fn keymap_set(
    app: AppHandle,
    action: String,
    accel: Option<String>,
) -> Result<(), IpcError> {
    let text = read_config_text(&app)?;
    let next = crate::keymap::set_binding(&text, &action, accel.as_deref());
    write_config_text(&app, &next)
}

/// Drop the whole `[keymap]` table (all actions back to defaults).
#[tauri::command]
pub fn keymap_reset(app: AppHandle) -> Result<(), IpcError> {
    let text = read_config_text(&app)?;
    write_config_text(&app, &crate::keymap::reset_keymap(&text))
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
    pub(crate) fn new(doc: Arc<Mutex<DocFile>>) -> Self {
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

/// Validate a CAN source request synchronously (spec §9): reject an unknown
/// `kind`, a `regex` source with no pattern, and an uncompilable pattern — all
/// before any worker/scan so a bad request rejects the promise rather than
/// streaming zero frames. Shared by `can_parse` and the CAN export commands.
pub(crate) fn validate_source_kind_regex(
    kind: &str,
    regex: &Option<String>,
) -> Result<(), IpcError> {
    match kind {
        "candump" | "asc" => Ok(()),
        "regex" => match regex {
            Some(p) => RegexSource::new(std::iter::empty::<Line>(), p)
                .map(|_| ())
                .map_err(IpcError::from_can),
            None => Err(IpcError::new("bad_regex", "regex source requires a pattern")),
        },
        other => Err(IpcError::new(
            "source_format",
            format!("unknown source kind '{other}'"),
        )),
    }
}

/// Build a `FrameSource` over `lines` for the given source kind. `validate_source_kind_regex`
/// must have run first (the regex is compile-checked there). Created and consumed
/// entirely on one blocking thread, so the returned `Box<dyn FrameSource>` never
/// needs to be `Send`. Shared by `can_parse` and the export commands.
pub(crate) fn build_frame_source(
    kind: &str,
    regex: &Option<String>,
    lines: DocLines,
) -> Result<Box<dyn FrameSource>, IpcError> {
    match kind {
        "asc" => Ok(Box::new(AscSource::new(lines))),
        "regex" => {
            let pat = regex
                .as_ref()
                .ok_or_else(|| IpcError::new("bad_regex", "regex source requires a pattern"))?;
            Ok(Box::new(
                RegexSource::new(lines, pat).map_err(IpcError::from_can)?,
            ))
        }
        "candump" => Ok(Box::new(CandumpSource::new(lines))),
        other => Err(IpcError::new(
            "source_format",
            format!("unknown source kind '{other}'"),
        )),
    }
}

#[cfg(test)]
mod source_validation_tests {
    use super::*;

    #[test]
    fn candump_and_asc_ok() {
        assert!(validate_source_kind_regex("candump", &None).is_ok());
        assert!(validate_source_kind_regex("asc", &None).is_ok());
    }

    #[test]
    fn regex_needs_pattern() {
        let err = validate_source_kind_regex("regex", &None).unwrap_err();
        assert_eq!(err.code, "bad_regex");
    }

    #[test]
    fn regex_valid_pattern_ok() {
        // RegexSource requires the `id` and `data` named groups; a complete
        // pattern must validate.
        assert!(
            validate_source_kind_regex("regex", &Some(r"(?<id>[0-9A-Fa-f]+)#(?<data>[0-9A-Fa-f]+)".to_string()))
                .is_ok()
        );
    }

    #[test]
    fn regex_invalid_pattern_rejected() {
        // "(" is an invalid regex; the code comes from lumelog-can's error map
        // (bad_regex / regex_compile) — assert it's a rejection, not a specific code.
        assert!(validate_source_kind_regex("regex", &Some("(".to_string())).is_err());
    }

    #[test]
    fn unknown_kind_rejected() {
        let err = validate_source_kind_regex("xyz", &None).unwrap_err();
        assert_eq!(err.code, "source_format");
    }
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
    validate_source_kind_regex(&source_kind, &regex).map_err(fail)?;
    let db_arc = dbc_id.and_then(|i| state.get_dbc(i));
    // `AppHandle` is `'static` + `Clone`, so the worker can re-fetch the managed
    // `AppDocs` to `end_search` when it finishes (a borrowed `State<'_>` cannot
    // cross into the `'static` blocking closure).
    let kind = source_kind.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let lines = DocLines::new(doc_arc);
        // The `Box<dyn FrameSource>` is created and consumed entirely on this one
        // blocking thread, so it never needs to be `Send` across the boundary.
        let mut src = build_frame_source(&kind, &regex, lines).expect("validated in prologue");
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
        if let Some(ranges) = lf.keep(line1 - 1, &text) {
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
#[allow(clippy::too_many_arguments)] // IPC: doc + conditions + case + time/lines + 2 channels + handles
pub fn filter_run(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
    conditions: Vec<FilterCondition>,
    case_sensitive: bool,
    time: Option<TimeSpecJson>,
    line_from: Option<u64>,
    line_to: Option<u64>,
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
    let line_range = match (line_from, line_to) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    };
    let time_core = time.as_ref().map(|t| t.to_core());
    if specs.is_empty() && time_core.is_none() && line_range.is_none() {
        // No axis supplied: defensive no-op (the panel guards this), never fan
        // out into a whole-file scan that keeps every line.
        let _ = on_done.send(FilterStats {
            kept: 0,
            scanned: 0,
            truncated: false,
        });
        state.end_search(req_id);
        return Ok(req_id);
    }
    let lf = LineFilter::build(specs, case_sensitive, time_core, line_range)
        .map_err(|e| fail(IpcError::from_core(e)))?;
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

/// Open the export target so a bad path rejects the command synchronously
/// (never spin up a worker that streams progress into a file we can't create).
fn open_export_target(path: &str) -> Result<std::fs::File, IpcError> {
    std::fs::File::create(path).map_err(|e| IpcError::new("io", e.to_string()))
}

/// Export the lines a filter/timeline/line-range selects, as raw source bytes
/// (exact encoding + line endings), to `target_path` (spec §7.5). Async like
/// `filter_run`: validate + create the file in the prologue, then scan+write on
/// a blocking task and return `req_id` immediately so `export_cancel` works.
/// Read/write errors are absorbed into `ExportStats::errored`; `on_done` always
/// fires with the final tally.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // IPC: doc + conditions + case + time/lines + path + 2 channels + handles
pub fn export_lines(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
    conditions: Vec<FilterCondition>,
    case_sensitive: bool,
    time: Option<TimeSpecJson>,
    line_from: Option<u64>,
    line_to: Option<u64>,
    target_path: String,
    on_progress: Channel<ExportProgress>,
    on_done: Channel<ExportStatsJson>,
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
    let line_range = match (line_from, line_to) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    };
    let time_core = time.as_ref().map(|t| t.to_core());
    let lf = LineFilter::build(specs, case_sensitive, time_core, line_range)
        .map_err(|e| fail(IpcError::from_core(e)))?;
    let file = open_export_target(&target_path).map_err(fail)?;
    tauri::async_runtime::spawn_blocking(move || {
        // Export holds the doc lock for the WHOLE run (coarser than filter_run /
        // can_parse, which lock per chunk via DocLines): the main thread stays
        // free and `export_cancel` — via the separate cancel registry — still stops
        // it promptly; partial bytes remain on `errored`/`cancelled`. Concurrent
        // reads of the SAME doc block until the export finishes.
        let mut doc = doc_arc.lock().unwrap_or_else(|p| p.into_inner());
        let mut writer = BufWriter::new(file);
        let mut prog = |written: u64| {
            let _ = on_progress.send(ExportProgress { written });
        };
        let stats =
            export::run(&mut doc, &lf, &mut writer, &mut prog, &|| cancel.load(Ordering::Relaxed));
        let _ = writer.flush();
        let _ = on_done.send(stats.into());
        app.state::<AppDocs>().end_search(req_id);
    });
    Ok(req_id)
}

/// Ask an in-flight `export_lines` to stop (shares the generic cancel registry).
#[tauri::command]
pub fn export_cancel(state: tauri::State<'_, AppDocs>, request_id: u64) -> Result<(), IpcError> {
    if state.cancel(request_id) {
        Ok(())
    } else {
        Err(IpcError::new(
            "unknown_request",
            format!("no in-flight job {request_id}"),
        ))
    }
}

/// Shared body of `export_csv`/`export_jsonl` (spec §5): validate + create the
/// target synchronously (bad path -> `io`, never a half-written worker), then
/// scan+write on a blocking task and return `req_id` immediately so
/// `export_can_cancel` works. `wide` picks CSV vs JSONL.
#[allow(clippy::too_many_arguments)] // IPC: doc + source + filter + path + 2 channels + handles
fn run_can_export(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
    dbc_id: Option<u64>,
    source_kind: String,
    regex: Option<String>,
    filter: CanFilterJson,
    target_path: String,
    on_progress: Channel<CanExportProgress>,
    on_done: Channel<CanExportStatsJson>,
    wide: bool,
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
    validate_source_kind_regex(&source_kind, &regex).map_err(fail)?;
    let db_arc = dbc_id.and_then(|i| state.get_dbc(i));
    let file = open_export_target(&target_path).map_err(fail)?;
    let kind = source_kind.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut writer = BufWriter::new(file);
        let mut prog = |written: u64| {
            let _ = on_progress.send(CanExportProgress { written });
        };
        let stats = export_can(
            &doc_arc,
            db_arc.as_deref(),
            &kind,
            &regex,
            &filter,
            &mut writer,
            &mut prog,
            &|| cancel.load(Ordering::Relaxed),
            wide,
        );
        let _ = writer.flush();
        let _ = on_done.send(stats.into());
        app.state::<AppDocs>().end_search(req_id);
    });
    Ok(req_id)
}

/// Export the CanPanel-filtered frame set as a wide CSV (raw re-decode, spec §7.5).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn export_csv(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
    dbc_id: Option<u64>,
    source_kind: String,
    regex: Option<String>,
    filter: CanFilterJson,
    target_path: String,
    on_progress: Channel<CanExportProgress>,
    on_done: Channel<CanExportStatsJson>,
) -> Result<u64, IpcError> {
    run_can_export(
        state,
        app,
        doc_id,
        dbc_id,
        source_kind,
        regex,
        filter,
        target_path,
        on_progress,
        on_done,
        true,
    )
}

/// Export the CanPanel-filtered frame set as JSONL (one `FrameJson` per line).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn export_jsonl(
    state: tauri::State<'_, AppDocs>,
    app: AppHandle,
    doc_id: u64,
    dbc_id: Option<u64>,
    source_kind: String,
    regex: Option<String>,
    filter: CanFilterJson,
    target_path: String,
    on_progress: Channel<CanExportProgress>,
    on_done: Channel<CanExportStatsJson>,
) -> Result<u64, IpcError> {
    run_can_export(
        state,
        app,
        doc_id,
        dbc_id,
        source_kind,
        regex,
        filter,
        target_path,
        on_progress,
        on_done,
        false,
    )
}

/// Ask an in-flight `export_csv`/`export_jsonl` to stop (shared cancel registry).
#[tauri::command]
pub fn export_can_cancel(
    state: tauri::State<'_, AppDocs>,
    request_id: u64,
) -> Result<(), IpcError> {
    if state.cancel(request_id) {
        Ok(())
    } else {
        Err(IpcError::new(
            "unknown_request",
            format!("no in-flight job {request_id}"),
        ))
    }
}

// ---- Crash-recovery shadow store + minimal config switch (M3 B1) ------------

/// LumeLog's config root: `<app_config>/LumeLog`.
fn recovery_root(app: &AppHandle) -> Result<std::path::PathBuf, IpcError> {
    let base = app
        .path()
        .app_config_dir()
        .map_err(|e| IpcError::new("io", format!("no config dir: {e}")))?;
    Ok(base.join("LumeLog"))
}

fn recovery_dir(app: &AppHandle) -> Result<std::path::PathBuf, IpcError> {
    Ok(recovery_root(app)?.join("recovery"))
}

fn config_path(app: &AppHandle) -> Result<std::path::PathBuf, IpcError> {
    Ok(recovery_root(app)?.join("config.toml"))
}

/// Current `recovery_enabled` (missing/unreadable config -> true).
fn recovery_enabled_now(app: &AppHandle) -> bool {
    let text = read_config_text(app).unwrap_or_default();
    crate::recovery::read_recovery_enabled(&text)
}

/// Persist a dirty doc's shadow (no-op when recovery is disabled).
#[tauri::command]
pub fn recovery_save(
    app: AppHandle,
    path: String,
    encoding: String,
    eol: String,
    text: String,
) -> Result<(), IpcError> {
    if !recovery_enabled_now(&app) {
        return Ok(()); // defence in depth; the front-end also gates scheduling
    }
    let dir = recovery_dir(&app)?;
    let saved_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let meta = crate::recovery::RecoveryMeta {
        path,
        encoding,
        eol,
        saved_at_ms,
    };
    crate::recovery::write_shadow(&dir, &meta, &text)
        .map_err(|e| IpcError::new("io", format!("write shadow: {e}")))
}

/// List recoverable shadows (empty when recovery is disabled).
#[tauri::command]
pub fn recovery_list(app: AppHandle) -> Result<Vec<crate::recovery::RecoveryEntry>, IpcError> {
    if !recovery_enabled_now(&app) {
        return Ok(Vec::new());
    }
    Ok(crate::recovery::list_shadows(&recovery_dir(&app)?))
}

/// Load one shadow's text + metadata.
#[tauri::command]
pub fn recovery_load(
    app: AppHandle,
    path: String,
) -> Result<crate::recovery::RecoveryContent, IpcError> {
    let dir = recovery_dir(&app)?;
    let (meta, text) = crate::recovery::read_shadow(&dir, &path)
        .map_err(|e| IpcError::new("io", format!("read shadow: {e}")))?;
    Ok(crate::recovery::RecoveryContent {
        path: meta.path,
        encoding: meta.encoding,
        eol: meta.eol,
        text,
    })
}

/// Delete a shadow (idempotent: a missing file is fine; other I/O errors surface).
#[tauri::command]
pub fn recovery_discard(app: AppHandle, path: String) -> Result<(), IpcError> {
    crate::recovery::discard_shadow(&recovery_dir(&app)?, &path)
        .map_err(|e| IpcError::new("io", format!("discard shadow: {e}")))
}

/// Read the minimal config switch.
#[tauri::command]
pub fn recovery_enabled(app: AppHandle) -> Result<bool, IpcError> {
    Ok(recovery_enabled_now(&app))
}

/// Write `recovery_enabled` into `config.toml`, preserving other keys.
#[tauri::command]
pub fn recovery_set_enabled(app: AppHandle, enabled: bool) -> Result<(), IpcError> {
    let text = read_config_text(&app)?;
    let next = crate::recovery::set_recovery_enabled(&text, enabled);
    write_config_text(&app, &next)
}

#[cfg(test)]
mod reload_tests {
    use super::*;

    #[test]
    fn reload_after_external_rewrite() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(b"a\nb\nc\n").unwrap();
        f.flush().unwrap();
        let path = f.path().to_path_buf();
        let state = AppDocs::default();
        let id = state.insert(DocFile::open(&path, 1 << 20).unwrap());

        // External rewrite: five lines.
        {
            let mut w = std::fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            w.write_all(b"1\n2\n3\n4\n5").unwrap(); // no trailing \n -> 5 lines
            w.flush().unwrap();
        }

        let info = reload_doc(&state, id).unwrap();
        assert_eq!(info.total_lines, Some(5));
        let text = {
            let arc = state.get(id).unwrap();
            let mut d = arc.lock().unwrap();
            d.chunk_at(0, 5).unwrap().text
        };
        assert!(text.contains('5'), "{text}");
        let disk_len = std::fs::metadata(&path).unwrap().len();
        let snap_len = state.get(id).unwrap().lock().unwrap().disk_meta().0;
        assert_eq!(snap_len, disk_len, "snapshot refreshed after reload");
    }

    #[test]
    fn reload_unknown_doc_is_unknown_doc() {
        let state = AppDocs::default();
        let err = reload_doc(&state, 42).unwrap_err();
        assert_eq!(err.code, "unknown_doc");
    }
}

#[cfg(test)]
mod filter_run_tests {
    use super::*;
    use lumelog_core::filter::{CondKind, CondMode, CondSpec, TimeFormat, TimeSpec};

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
        LineFilter::build(specs, case, None, None).unwrap()
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

    #[test]
    fn scan_filter_line_range_degrade() {
        let lf = LineFilter::build(
            vec![CondSpec { mode: CondMode::Include, kind: CondKind::Literal, text: "x".into() }],
            true,
            None,
            Some((2, 3)),
        )
        .unwrap();
        let mut got: Vec<LineHit> = Vec::new();
        scan_filter(
            lines("x\nx\nx\nx").into_iter(),
            &lf,
            &mut |b| got.extend(b),
            &|| false,
            usize::MAX,
        );
        assert_eq!(
            got.iter().map(|h| h.line).collect::<Vec<_>>(),
            vec![1, 2],
            "only 1-based lines 2,3 survive (0-based 1,2)"
        );
    }

    #[test]
    fn scan_filter_time_axis_keeps_range() {
        let t = TimeSpec {
            extract: r"(?<ts>\d+)".to_string(),
            format: TimeFormat::EpochMillis,
            from: Some("2".into()),
            to: Some("3".into()),
            keep_when_no_ts: false,
        };
        let lf = LineFilter::build(vec![], true, Some(t), None).unwrap();
        let mut got: Vec<LineHit> = Vec::new();
        scan_filter(
            lines("ts=1\nts=2\nts=3\nts=4").into_iter(),
            &lf,
            &mut |b| got.extend(b),
            &|| false,
            usize::MAX,
        );
        assert_eq!(got.iter().map(|h| h.line).collect::<Vec<_>>(), vec![1, 2]);
    }
}

#[cfg(test)]
mod export_cmd_tests {
    use super::*;

    #[test]
    fn open_export_target_bad_path_is_io() {
        // An empty path cannot be created on any OS -> `io`, synchronously.
        let err = open_export_target("").unwrap_err();
        assert_eq!(err.code, "io");
    }

    #[test]
    fn open_export_target_creates_file() {
        let p = std::env::temp_dir().join(format!("lumelog_exp_ok_{}.log", std::process::id()));
        let f = open_export_target(p.to_str().unwrap());
        assert!(f.is_ok(), "valid temp path creates the target");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn can_export_stats_json_from_and_camel_case() {
        let json: CanExportStatsJson = crate::can_export::CanExportStats {
            written: 2,
            scanned: 5,
            cancelled: true,
            errored: false,
        }
        .into();
        let v = serde_json::to_value(&json).unwrap();
        assert_eq!(v.get("written").unwrap(), 2);
        assert_eq!(v.get("scanned").unwrap(), 5);
        assert_eq!(v.get("cancelled").unwrap(), true);
        assert_eq!(v.get("errored").unwrap(), false);
    }
}

/// Gather decoded text for a sparse, ascending set of 0-based line indices.
/// Each line is fetched with `chunk_at(i, 1)` and stripped of its trailing
/// terminator exactly like the front end's `loadWindow` (`\n` then `\r`), so the
/// cu16 highlight ranges `filter_run` produced line up with this text
/// character-for-character. Only the requested lines are resident — never the
/// whole file (spec §3). Bounded by `MAX_CHUNK_LINES` per call.
fn gather_lines(doc: &mut DocFile, indices: &[u32]) -> Result<LineContentPayload, IpcError> {
    if indices.len() > MAX_CHUNK_LINES as usize {
        return Err(IpcError::new(
            "invalid_argument",
            format!(
                "too many lines requested ({} > {MAX_CHUNK_LINES})",
                indices.len()
            ),
        ));
    }
    let mut lines = Vec::with_capacity(indices.len());
    let mut invalid = 0usize;
    for &idx in indices {
        let chunk = doc.chunk_at(idx as usize, 1).map_err(IpcError::from_core)?;
        invalid += chunk.invalid_count;
        let mut text = chunk.text;
        if text.ends_with('\n') {
            text.pop();
        }
        if text.ends_with('\r') {
            text.pop();
        }
        lines.push(IndexedText { index: idx, text });
    }
    Ok(LineContentPayload {
        lines,
        invalid_count: invalid as u64,
    })
}

/// Fetch the decoded text for the visible window of a filtered read-only list
/// (spec §3.2). The whole file is never resident: the front end requests only
/// the original line numbers currently shown, and text flows back over a
/// channel like `doc_chunk`.
#[tauri::command]
pub fn doc_lines_at(
    state: tauri::State<'_, AppDocs>,
    doc_id: u64,
    indices: Vec<u32>,
    on_data: Channel<LineContentPayload>,
) -> Result<(), IpcError> {
    let doc_arc = match state.get(doc_id) {
        Some(a) => a,
        None => return Err(IpcError::new("unknown_doc", format!("no document {doc_id}"))),
    };
    let mut doc = doc_arc
        .lock()
        .map_err(|_| IpcError::new("io", "document mutex poisoned"))?;
    let payload = gather_lines(&mut doc, &indices)?;
    let _ = on_data.send(payload);
    Ok(())
}

#[cfg(test)]
mod doc_lines_at_tests {
    use super::*;
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
    fn gather_lines_returns_requested_subset() {
        let (_k, mut doc) = doc_of("a\nbb\nccc\n");
        let p = gather_lines(&mut doc, &[0, 2]).unwrap();
        assert_eq!(p.lines.len(), 2);
        assert_eq!(p.lines[0].index, 0);
        assert_eq!(p.lines[0].text, "a");
        assert_eq!(p.lines[1].index, 2);
        assert_eq!(p.lines[1].text, "ccc", "terminator stripped, original index kept");
    }

    #[test]
    fn gather_lines_strips_crlf() {
        let (_k, mut doc) = doc_of("one\r\ntwo\r\n");
        let p = gather_lines(&mut doc, &[1]).unwrap();
        assert_eq!(p.lines[0].text, "two", "no trailing CR");
    }

    #[test]
    fn gather_lines_over_cap_rejects() {
        let (_k, mut doc) = doc_of("x\n");
        let idx: Vec<u32> = (0..=MAX_CHUNK_LINES).collect();
        let err = gather_lines(&mut doc, &idx).unwrap_err();
        assert_eq!(err.code, "invalid_argument");
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
