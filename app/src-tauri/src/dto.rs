//! Serde data-transfer objects crossing the IPC boundary, aligned with spec §9.
//! `char_map` deliberately stays inside the core (the front end renders text;
//! in-line coordinate mapping returns with the Plan 3 search UI). `IpcError`
//! carries a stable machine `code` plus a human `message` and optional `detail`.

use lumelog_can::dbc::Database;
use lumelog_can::decode::decode_signals;
use lumelog_can::error::CanError;
use lumelog_can::frame::{Frame, SignalValue};
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
            CoreError::Conflict {
                disk: (size, _),
                unmappable,
            } => {
                // One informed confirmation: fold the lossy-encode consequence
                // into the conflict detail so a single force prompt discloses
                // both gates the plan's single knob couples (final-review I-1).
                let detail = if *unmappable > 0 {
                    format!(
                        "磁盘现为 {size} 字节，且本次还将把 {unmappable} 处无法表示的字符写为 ?（不可恢复）"
                    )
                } else {
                    format!("磁盘现为 {size} 字节")
                };
                ("conflict", Some(detail))
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

    /// Map a CAN engine error to a stable IPC code (spec §9). The five codes
    /// mirror `CanError::ipc_code()` — the frontend's Chinese copy table keys off
    /// exactly these strings, so a rename here MUST land in `ipc.ts` in tandem.
    pub fn from_can(e: CanError) -> Self {
        let (code, detail) = match &e {
            CanError::Io(_) => ("io", None),
            CanError::DbcSyntax {
                line,
                context,
                kind,
            } => ("dbc_syntax", Some(format!("第 {line} 行：{context}（{kind}）"))),
            CanError::BadRegex(m) => ("bad_regex", Some(m.clone())),
            CanError::RegexCompile(m) => ("regex_compile", Some(m.clone())),
            CanError::SourceFormat(m) => ("source_format", Some(m.clone())),
        };
        Self {
            code: code.to_string(),
            message: e.to_string(),
            detail,
        }
    }
}

// ---- CAN DTOs (spec §7.4 / §9). camelCase to match ipc.ts ------------------

/// One decoded signal within a frame (mirrors core `SignalValue`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalValueJson {
    pub name: String,
    pub raw: i64,
    pub physical: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_text: Option<String>,
    pub unit: String,
}

/// One frame + its optional DBC decoding, streamed to the CanPanel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameJson {
    pub line_number: u64,
    /// Milliseconds; core carries seconds. `None` when the line has no timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<f64>,
    pub channel: u8,
    pub can_id: u32,
    pub can_id_hex: String,
    pub dlc: u8,
    pub data_hex: String,
    pub is_remote: bool,
    pub is_fd: bool,
    pub error_frame: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_name: Option<String>,
    pub signals: Vec<SignalValueJson>,
}

/// A message row in the `dbc_import` summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbcMsgSummary {
    pub id: u32,
    pub name: String,
    pub dlc: u8,
    pub signal_count: u32,
}

/// A lenient-parse warning row (from `CanError::DbcSyntax`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbcErrorJson {
    pub line: u64,
    pub context: String,
    pub kind: String,
}

/// Outcome of `dbc_import`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DbcInfo {
    pub dbc_id: u64,
    pub messages: Vec<DbcMsgSummary>,
    pub errors: Vec<DbcErrorJson>,
}

/// Terminal `can_parse` tally.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanStatsJson {
    pub decoded: u64,
    pub skipped: u64,
}

fn signal_value_json(s: SignalValue) -> SignalValueJson {
    SignalValueJson {
        name: s.name,
        raw: s.raw,
        physical: s.physical,
        enum_text: s.enum_text,
        unit: s.unit,
    }
}

/// Build the wire frame: hex id, ms timestamp, uppercase payload hex, and — when
/// a DBC message matches by id — its decoded signals. Never panics; a frame with
/// no matching message just carries its raw hex (`signals: []`, `msg_name: None`).
pub fn frame_json(f: &Frame, db: Option<&Database>) -> FrameJson {
    let n = f.dlc as usize;
    let data_hex = f.data[..n]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<String>();
    let matched = db.and_then(|d| d.messages.iter().find(|m| m.id == f.can_id));
    let (msg_name, signals) = match matched {
        Some(m) => (
            Some(m.name.clone()),
            decode_signals(m, &f.data[..n])
                .into_iter()
                .map(signal_value_json)
                .collect(),
        ),
        None => (None, Vec::new()),
    };
    FrameJson {
        line_number: f.line_number,
        timestamp_ms: f.timestamp.map(|s| s * 1000.0),
        channel: f.channel,
        can_id: f.can_id,
        can_id_hex: format!("0x{:X}", f.can_id),
        dlc: f.dlc,
        data_hex,
        is_remote: f.flags.is_remote,
        is_fd: f.flags.is_fd,
        error_frame: f.flags.is_error_frame,
        msg_name,
        signals,
    }
}

#[cfg(test)]
mod can_tests {
    use super::*;
    use lumelog_can::frame::FrameFlags;

    fn sample_db() -> Database {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/lumelog-can/tests/data/sample.dbc"
        );
        let src = std::fs::read_to_string(path).unwrap();
        // sample.dbc has a deliberately broken SG_ line, so strict `parse`
        // errors — the lenient path yields the good messages + a warning.
        lumelog_can::dbc::parse_lenient(&src).database
    }

    fn frame(can_id: u32, data: [u8; 64], dlc: u8, ts: Option<f64>, flags: FrameFlags) -> Frame {
        Frame {
            line_number: 3,
            timestamp: ts,
            channel: 1,
            can_id,
            dlc,
            data,
            flags,
        }
    }

    #[test]
    fn frame_json_maps_fields() {
        let mut data = [0u8; 64];
        for (i, b) in [0xDEu8, 0xAD, 0xBE, 0xEF, 0x12, 0x34, 0x56, 0x78]
            .iter()
            .enumerate()
        {
            data[i] = *b;
        }
        let f = frame(0x1A0, data, 8, Some(1.5), FrameFlags::default());
        let j = frame_json(&f, None);
        assert_eq!(j.can_id_hex, "0x1A0");
        assert_eq!(j.data_hex, "DEADBEEF12345678");
        assert_eq!(j.timestamp_ms, Some(1500.0));
        assert!(j.signals.is_empty());
        assert_eq!(j.msg_name, None);
    }

    #[test]
    fn frame_json_decodes_signals_when_matched() {
        let db = sample_db();
        let mut data = [0u8; 64];
        // EngineData id 291: EngineSpeed 16@0 Intel factor .125, CoolantTemp 8@16 signed -40
        data[0] = 0x00;
        data[1] = 0x10; // EngineSpeed raw = 0x1000 = 4096 -> phys 512.0
        data[2] = 90; // CoolantTemp raw = 90 -> phys 50.0, VAL_ 90 -> "Normal"
        let f = frame(291, data, 8, None, FrameFlags::default());
        let j = frame_json(&f, Some(&db));
        assert_eq!(j.msg_name.as_deref(), Some("EngineData"));
        let speed = j.signals.iter().find(|s| s.name == "EngineSpeed").unwrap();
        assert_eq!(speed.raw, 4096);
        assert!((speed.physical - 512.0).abs() < 1e-9);
        let temp = j.signals.iter().find(|s| s.name == "CoolantTemp").unwrap();
        assert!((temp.physical - 50.0).abs() < 1e-9);
        assert_eq!(temp.enum_text.as_deref(), Some("Normal"));
        assert_eq!(temp.unit, "degC");
    }

    #[test]
    fn error_frame_has_no_signals_and_empty_data() {
        let flags = FrameFlags {
            is_error_frame: true,
            ..Default::default()
        };
        let f = frame(0, [0u8; 64], 0, None, flags);
        let j = frame_json(&f, None);
        assert!(j.data_hex.is_empty());
        assert!(j.signals.is_empty());
        assert!(j.error_frame);
    }

    #[test]
    fn from_can_codes_are_stable() {
        let cases = [
            (CanError::Io(std::io::Error::other("x")), "io"),
            (
                CanError::DbcSyntax {
                    line: 7,
                    context: "SG_ ...".into(),
                    kind: "bad SG_".into(),
                },
                "dbc_syntax",
            ),
            (CanError::BadRegex("r".into()), "bad_regex"),
            (CanError::RegexCompile("r".into()), "regex_compile"),
            (CanError::SourceFormat("s".into()), "source_format"),
        ];
        for (err, code) in cases {
            assert_eq!(IpcError::from_can(err).code, code);
        }
    }

    #[test]
    fn frame_json_serializes_camel_case() {
        let f = frame(0x100, [0u8; 64], 0, Some(2.0), FrameFlags::default());
        let v = serde_json::to_value(frame_json(&f, None)).unwrap();
        assert!(v.get("lineNumber").is_some());
        assert!(v.get("canIdHex").is_some());
        assert!(v.get("timestampMs").is_some());
        assert!(v.get("isRemote").is_some());
        assert!(v.get("errorFrame").is_some());
    }
}
