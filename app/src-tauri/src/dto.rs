//! Serde data-transfer objects crossing the IPC boundary, aligned with spec §9.
//! `char_map` deliberately stays inside the core (the front end renders text;
//! in-line coordinate mapping returns with the Plan 3 search UI). `IpcError`
//! carries a stable machine `code` plus a human `message` and optional `detail`.

use lumelog_can::dbc::Database;
use lumelog_can::decode::decode_signals;
use lumelog_can::error::CanError;
use lumelog_can::frame::{Frame, SignalValue};
use lumelog_core::error::{CoreError, OpenReject};
use lumelog_core::export::ExportStats as CoreExportStats;
use crate::can_export::CanExportStats;
use lumelog_core::filter::{CondKind, CondMode, CondSpec, Cu16Range, TimeFormat, TimeSpec};
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
    /// Lines visible in the (partial) index at tick time.
    pub indexed_lines: u64,
    pub done: bool,
}

/// One Find-in-Files hit (Plan 44); `path` is relative to the scan root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FifHitJson {
    pub path: String,
    pub line: u64,
    pub text: String,
}

/// Streaming batch: hits during the scan, `stats` set on the final event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FifBatch {
    #[serde(default)]
    pub hits: Vec<FifHitJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<FifStatsJson>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FifStatsJson {
    pub files: u64,
    pub skipped: u64,
    pub hits: u64,
    pub truncated: bool,
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

// ---- CAN export DTOs (spec §7.5 / §9). camelCase to match ipc.ts -------------

/// Signal comparison operator for the CanPanel signal condition (mirrors the
/// `signalOp` string values used by `canfilter.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalOp {
    #[serde(rename = ">")]
    Gt,
    #[serde(rename = "<")]
    Lt,
    #[serde(rename = ">=")]
    Ge,
    #[serde(rename = "<=")]
    Le,
    #[serde(rename = "==")]
    Eq,
}

/// The CanPanel view filter, mirrored server-side so `export_csv`/`export_jsonl`
/// reproduce exactly what the panel shows. Field-for-field equal to
/// `canfilter.ts`'s `CanFilter`; every axis optional (absent = not applied).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CanFilterJson {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_from: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_to: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_op: Option<SignalOp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_from_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_ms: Option<f64>,
}

/// Throttled CAN-export progress (`export_csv`/`export_jsonl` `on_progress`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanExportProgress {
    pub written: u64,
}

/// Terminal CAN-export tally (`on_done`), mirroring core `CanExportStats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanExportStatsJson {
    pub written: u64,
    pub scanned: u64,
    pub cancelled: bool,
    pub errored: bool,
}

impl From<CanExportStats> for CanExportStatsJson {
    fn from(s: CanExportStats) -> Self {
        CanExportStatsJson {
            written: s.written,
            scanned: s.scanned,
            cancelled: s.cancelled,
            errored: s.errored,
        }
    }
}

/// Result of a tail poll (`doc_tail_append`, Roadmap: tail 跟随).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailAppendJson {
    /// The file was truncated/replaced: the caller must `doc_reload`.
    pub rotated: bool,
    pub lines_added: u64,
    pub total_lines: u64,
    pub size: u64,
}

/// Result of a BLF parse run (`can_parse_blf`, Roadmap: BLF).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlfStatsJson {
    pub decoded: u64,
    /// Unsupported/unknown objects skipped (LIN, statistics, ...).
    pub skipped: u64,
    /// Fatal parse error after the stream stopped (frames already sent stay).
    pub error: Option<String>,
}

// ---- M3 filter DTOs (spec §6.4 / §9). camelCase to match ipc.ts ------------

/// One filter condition as it arrives from the query panel. `mode` is
/// `"include"`/`"exclude"`, `kind` is `"literal"`/`"regex"`; unknown strings
/// degrade to include/literal rather than erroring (the panel only emits the
/// four known values, and a typo must not abort the whole scan).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCondition {
    pub mode: String,
    pub kind: String,
    pub text: String,
}

impl FilterCondition {
    pub fn to_core(&self) -> CondSpec {
        CondSpec {
            mode: if self.mode.eq_ignore_ascii_case("exclude") {
                CondMode::Exclude
            } else {
                CondMode::Include
            },
            kind: if self.kind.eq_ignore_ascii_case("regex") {
                CondKind::Regex
            } else {
                CondKind::Literal
            },
            text: self.text.clone(),
        }
    }
}

/// A highlight span in UTF-16 code units within a line (front-end `String.slice`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cu16RangeJson {
    pub cu16_start: u32,
    pub cu16_len: u32,
}

impl From<Cu16Range> for Cu16RangeJson {
    fn from(r: Cu16Range) -> Self {
        Cu16RangeJson { cu16_start: r.start, cu16_len: r.len }
    }
}

/// One kept line + its include-match highlight ranges, batched (`on_batch`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineHit {
    pub line: u64,
    pub ranges: Vec<Cu16RangeJson>,
}

/// Terminal `filter_run` tally (`on_done`). `truncated` is set when range output
/// was capped at `HIT_RANGE_CAP` (later kept lines carry empty `ranges`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterStats {
    pub kept: u64,
    pub scanned: u64,
    pub truncated: bool,
}

/// One requested line's decoded text, keyed by its original 0-based index.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedText {
    pub index: u32,
    pub text: String,
}

/// Payload of `doc_lines_at` (`on_data`) — a sparse subset of line texts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineContentPayload {
    pub lines: Vec<IndexedText>,
    pub invalid_count: u64,
}

// ---- M3 timeline + export DTOs (spec §3 / §9). camelCase to match ipc.ts ------

/// Timeline constraint for `filter_run`/`export_lines`: a regex with a named
/// group `ts` + a built-in format (`iso`/`epoch_s`/`epoch_ms`) + optional
/// `[from,to]` bound strings parsed by that format, and whether lines whose `ts`
/// is absent survive the window.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeSpecJson {
    pub extract_pattern: String,
    pub format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub keep_when_no_ts: bool,
}

impl TimeSpecJson {
    pub fn to_core(&self) -> TimeSpec {
        TimeSpec {
            extract: self.extract_pattern.clone(),
            format: match self.format.as_str() {
                "epoch_s" => TimeFormat::EpochSec,
                "epoch_ms" => TimeFormat::EpochMillis,
                _ => TimeFormat::Iso,
            },
            from: self.from.clone(),
            to: self.to.clone(),
            keep_when_no_ts: self.keep_when_no_ts,
        }
    }
}

/// Throttled export progress (`on_progress`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgress {
    pub written: u64,
}

/// Terminal `export_lines` tally (`on_done`), mirroring core `ExportStats`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStatsJson {
    pub written: u64,
    pub scanned: u64,
    pub cancelled: bool,
    pub errored: bool,
}

impl From<CoreExportStats> for ExportStatsJson {
    fn from(s: CoreExportStats) -> Self {
        ExportStatsJson {
            written: s.written,
            scanned: s.scanned,
            cancelled: s.cancelled,
            errored: s.errored,
        }
    }
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
    let n = (f.dlc as usize).min(f.data.len());
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
mod blf_stats_tests {
    use super::*;

    #[test]
    fn blf_stats_camel_case() {
        let ok = serde_json::to_value(BlfStatsJson {
            decoded: 12,
            skipped: 3,
            error: None,
        })
        .unwrap();
        assert_eq!(ok["decoded"], 12);
        assert_eq!(ok["skipped"], 3);
        assert!(ok["error"].is_null());

        let bad = serde_json::to_value(BlfStatsJson {
            decoded: 1,
            skipped: 0,
            error: Some("blf: bad".into()),
        })
        .unwrap();
        assert_eq!(bad["error"], "blf: bad");
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

#[cfg(test)]
mod filter_dto_tests {
    use super::*;

    #[test]
    fn filter_condition_deserializes_and_maps_to_core() {
        let c: FilterCondition =
            serde_json::from_str(r#"{"mode":"exclude","kind":"regex","text":"a"}"#).unwrap();
        let spec = c.to_core();
        assert_eq!(spec.mode, CondMode::Exclude);
        assert_eq!(spec.kind, CondKind::Regex);
        assert_eq!(spec.text, "a");
        // unknown strings degrade to include/literal, never error
        let u: FilterCondition =
            serde_json::from_str(r#"{"mode":"x","kind":"y","text":"z"}"#).unwrap();
        assert_eq!(u.to_core().mode, CondMode::Include);
        assert_eq!(u.to_core().kind, CondKind::Literal);
    }

    #[test]
    fn line_hit_serializes_camel_case() {
        let hit = LineHit {
            line: 5,
            ranges: vec![Cu16RangeJson { cu16_start: 3, cu16_len: 2 }],
        };
        let v = serde_json::to_value(&hit).unwrap();
        assert_eq!(v.get("line").unwrap(), 5);
        let r = &v["ranges"][0];
        assert_eq!(r.get("cu16Start").unwrap(), 3);
        assert_eq!(r.get("cu16Len").unwrap(), 2);
    }

    #[test]
    fn filter_stats_shape() {
        let v = serde_json::to_value(FilterStats {
            kept: 1,
            scanned: 2,
            truncated: true,
        })
        .unwrap();
        assert_eq!(v.get("kept").unwrap(), 1);
        assert_eq!(v.get("scanned").unwrap(), 2);
        assert_eq!(v.get("truncated").unwrap(), true);
    }

    #[test]
    fn cu16_range_from_core() {
        let j: Cu16RangeJson = Cu16Range { start: 7, len: 4 }.into();
        assert_eq!((j.cu16_start, j.cu16_len), (7, 4));
    }
}

#[cfg(test)]
mod timeline_export_dto_tests {
    use super::*;

    #[test]
    fn time_spec_maps_formats() {
        let mk = |f: &str| TimeSpecJson {
            extract_pattern: "(?<ts>x)".into(),
            format: f.into(),
            from: None,
            to: Some("5".into()),
            keep_when_no_ts: true,
        };
        assert_eq!(mk("epoch_ms").to_core().format, TimeFormat::EpochMillis);
        assert_eq!(mk("epoch_s").to_core().format, TimeFormat::EpochSec);
        assert_eq!(mk("iso").to_core().format, TimeFormat::Iso);
        assert_eq!(mk("garbage").to_core().format, TimeFormat::Iso, "unknown -> Iso");
        assert_eq!(mk("iso").to_core().to.as_deref(), Some("5"));
    }

    #[test]
    fn export_stats_json_camel_case() {
        let v = serde_json::to_value(ExportStatsJson {
            written: 1,
            scanned: 2,
            cancelled: true,
            errored: false,
        })
        .unwrap();
        assert_eq!(v.get("written").unwrap(), 1);
        assert_eq!(v.get("scanned").unwrap(), 2);
        assert_eq!(v.get("cancelled").unwrap(), true);
        assert_eq!(v.get("errored").unwrap(), false);
    }

    #[test]
    fn export_stats_from_core_maps() {
        let j: ExportStatsJson = CoreExportStats {
            written: 3,
            scanned: 9,
            cancelled: false,
            errored: true,
        }
        .into();
        assert_eq!((j.written, j.scanned, j.cancelled, j.errored), (3, 9, false, true));
    }
}
