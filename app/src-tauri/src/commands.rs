//! Tauri v2 IPC commands. Each is a thin forwarder onto `lumelog-core`; no
//! document/encoding/search logic lives here (spec §3). Heavy calls run as
//! synchronous commands for Task 8 (verification = `cargo check`); the async /
//! non-blocking indexing model is exercised by the large-file UI in Task 10.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::UNIX_EPOCH;

use lumelog_core::doc::{save_text, DocFile, EolStyle, OverwritePolicy, SaveOptions};
use lumelog_core::encoding::EncodingLabel;
use lumelog_core::search::{search_stream, Mode, SearchQuery};
use tauri::ipc::Channel;

use crate::dto::*;
use crate::state::AppDocs;

/// Effectively-unlimited open cap; the 4 GiB u32 line-index ceiling is still
/// enforced inside the core builder (`CoreError::TooLarge`).
const SIZE_LIMIT: u64 = u64::MAX;
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

fn parse_eol(s: &str) -> Option<EolStyle> {
    match s.to_ascii_lowercase().as_str() {
        "lf" => Some(EolStyle::Lf),
        "crlf" => Some(EolStyle::Crlf),
        "mixed" => Some(EolStyle::Mixed),
        _ => None,
    }
}
