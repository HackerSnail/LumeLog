//! Serde data-transfer objects crossing the IPC boundary, aligned with spec §9.
//! `char_map` deliberately stays inside the core (the front end renders text;
//! in-line coordinate mapping returns with the Plan 3 search UI). `IpcError`
//! carries a stable machine `code` plus a human `message` and optional `detail`.

use lumelog_core::error::{CoreError, OpenReject};
use serde::{Deserialize, Serialize};

/// Summary returned by `doc_open`. Indexing is synchronous in v1, so
/// `total_lines` is `Some`; the field is `Option` to leave the §9 background
/// "estimate then exact" shape intact for the large-file path (Task 10).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocInfo {
    pub doc_id: u64,
    pub size: u64,
    pub mtime_millis: u64,
    pub encoding: String,
    /// Dominant line-ending style the core detected on open (spec §4.4). The
    /// front end MUST save back with this value (unless the user overrides it)
    /// so a CRLF bench log is not silently rewritten to LF.
    pub eol: String,
    /// Which view the front end should mount: `"edit"` (CodeMirror, ≤200MB)
    /// or `"readonly"` (virtual list, >200MB). Single source for the §4.3/A1
    /// threshold so the shell and front end cannot drift apart.
    pub mode: String,
    pub line_count_estimate: u64,
    pub total_lines: Option<u64>,
}

/// One decoded block, streamed to the front end over a `Channel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkPayload {
    pub start_line: u64,
    pub text: String,
    pub invalid_count: u64,
}

/// Progress of the background index build (`on_index_progress`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    pub doc_id: u64,
    pub indexed_bytes: u64,
    pub done: bool,
}

/// A single search hit, batched to the front end (`on_batch`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchJson {
    pub line: u64,
    pub byte_in_line: u64,
    pub len: u64,
    pub snippet: String,
}

/// Terminal search tally (`on_done`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchStatsJson {
    pub matches: u64,
    pub scanned_bytes: u64,
    pub cancelled: bool,
}

/// Outcome of `doc_save`, mirroring the core `SaveReport`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveReportJson {
    pub replace_count: u64,
    pub conflict: bool,
}

/// A row of the currently-open documents (`doc_list`, front-end restore).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocEntry {
    pub doc_id: u64,
    pub path: String,
    pub encoding: String,
    pub size: u64,
    pub total_lines: u64,
}

/// Structured IPC error. `code` is one of: `not_found`, `is_directory`,
/// `permission_denied`, `too_large`, `io`, `conflict`, `unmappable`, `locked`,
/// `line_out_of_range`, `encoding_unsupported`, `invalid_pattern`, `unknown_doc`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl IpcError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            detail: None,
        }
    }

    /// Map a core engine error to a stable IPC code (spec §9).
    pub fn from_core(e: CoreError) -> Self {
        let (code, detail) = match &e {
            CoreError::Io(_) => ("io", None),
            CoreError::TooLarge => ("too_large", None),
            CoreError::LineOutOfRange => ("line_out_of_range", None),
            CoreError::Encoding(_) => ("encoding_unsupported", None),
            CoreError::InvalidPattern(_) => ("invalid_pattern", None),
            CoreError::Conflict((size, _)) => {
                ("conflict", Some(format!("disk size now {size}")))
            }
            CoreError::Unmappable(n) => ("unmappable", Some(n.to_string())),
            CoreError::Locked => ("locked", None),
        };
        Self {
            code: code.to_string(),
            message: e.to_string(),
            detail,
        }
    }

    /// Map a pre-open validation rejection to an IPC code.
    pub fn from_open_reject(r: OpenReject) -> Self {
        let (code, detail) = match &r {
            OpenReject::NotFound => ("not_found", None),
            OpenReject::IsDirectory => ("is_directory", None),
            OpenReject::PermissionDenied => ("permission_denied", None),
            OpenReject::TooLarge(size, limit) => {
                ("too_large", Some(format!("{size} > {limit} bytes")))
            }
        };
        Self {
            code: code.to_string(),
            message: r.to_string(),
            detail,
        }
    }
}
