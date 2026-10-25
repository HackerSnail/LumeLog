//! CAN view-filter predicate + (later) export text builders — spec §7.5 / §9,
//! M3 increment 3. `frame_matches` is a faithful port of the front-end
//! `canfilter.ts` predicate and operates on the SAME `FrameJson` shape the panel
//! renders, so backend export can never drift from what the user sees.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

use lumelog_can::dbc::Database;
use lumelog_core::doc::DocFile;

use crate::commands::{build_frame_source, DocLines};
use crate::dto::{frame_json, CanFilterJson, FrameJson, SignalOp};

/// Qualified signal-column header shared by the CSV builder and the column index.
const CSV_META: [&str; 7] = [
    "lineNumber",
    "timestampMs",
    "channel",
    "canId",
    "msgName",
    "dlc",
    "dataHex",
];

/// Turn `(message, signals)` pairs into qualified `消息名.信号名` column names in
/// order, de-duplicating exact repeats with a `#n` suffix. Pure + testable
/// without constructing `Signal`s.
pub(crate) fn qualified_columns(msgs: &[(String, Vec<String>)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (msg, sigs) in msgs {
        for s in sigs {
            let mut name = format!("{msg}.{s}");
            if out.contains(&name) {
                let mut n = 2;
                while out.contains(&format!("{name}#{n}")) {
                    n += 1;
                }
                name = format!("{name}#{n}");
            }
            out.push(name);
        }
    }
    out
}

/// Signal column names from an optional DBC (empty when none imported).
pub fn db_signal_columns(db: Option<&Database>) -> Vec<String> {
    let pairs: Vec<(String, Vec<String>)> = db
        .map(|d| {
            d.messages
                .iter()
                .map(|m| (m.name.clone(), m.signals.iter().map(|s| s.name.clone()).collect()))
                .collect()
        })
        .unwrap_or_default();
    qualified_columns(&pairs)
}

/// Wrap a field in CSV quotes only when needed; double embedded quotes.
fn csv_escape(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Stable ASCII float rendering for physical values: `{:.6}` then trailing zeros
/// (and a lone trailing `.`) trimmed. Non-finite -> empty string.
fn fmt_physical(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    let s = format!("{v:.6}");
    let t = s.trim_end_matches('0').trim_end_matches('.');
    t.to_string()
}

/// CSV header line: 7 meta columns + one per DBC signal, CRLF-terminated.
pub fn csv_header(db: Option<&Database>) -> String {
    let mut cells: Vec<String> = CSV_META.iter().map(|s| s.to_string()).collect();
    cells.extend(db_signal_columns(db));
    let line = cells
        .iter()
        .map(|c| csv_escape(c))
        .collect::<Vec<_>>()
        .join(",");
    format!("{line}\r\n")
}

/// One CSV data row. `col_index` maps a qualified `消息名.信号名` to its column
/// offset (0-based within the signal-column block); `ncols` is the total column
/// count. Cells for signals the frame lacks stay empty. CRLF-terminated.
pub fn csv_row(fj: &FrameJson, col_index: &HashMap<String, usize>, ncols: usize) -> String {
    let mut cells = vec![String::new(); ncols];
    cells[0] = fj.line_number.to_string();
    cells[1] = fj.timestamp_ms.map(fmt_ts).unwrap_or_default();
    cells[2] = fj.channel.to_string();
    cells[3] = fj.can_id.to_string();
    cells[4] = fj.msg_name.clone().unwrap_or_default();
    cells[5] = fj.dlc.to_string();
    cells[6] = fj.data_hex.clone();
    if let Some(msg) = &fj.msg_name {
        for s in &fj.signals {
            let key = format!("{msg}.{}", s.name);
            if let Some(&col) = col_index.get(&key) {
                cells[col] = fmt_physical(s.physical);
            }
        }
    }
    let line = cells
        .iter()
        .map(|c| csv_escape(c))
        .collect::<Vec<_>>()
        .join(",");
    format!("{line}\r\n")
}

/// Timestamp cell: whole ms as integer-ish, else the trimmed float (ms domain).
fn fmt_ts(ms: f64) -> String {
    if ms.is_finite() && (ms - ms.round()).abs() < 1e-9 {
        format!("{}", ms.round() as i64)
    } else {
        fmt_physical(ms)
    }
}

/// One JSONL line: the frame serialized exactly like `can_parse` sends it.
pub fn jsonl_line(fj: &FrameJson) -> String {
    // FrameJson serializes to camelCase (matches the front-end shape), so a
    // failed to_string is unreachable; fall back to an empty object defensively.
    let s = serde_json::to_string(fj).unwrap_or_else(|_| "{}".to_string());
    format!("{s}\n")
}

/// Tally of one `export_can` run (mirrors the log `export_lines` shape).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanExportStats {
    pub written: u64,
    pub scanned: u64,
    pub cancelled: bool,
    pub errored: bool,
}

const CAN_EXPORT_BATCH: u64 = 512;

/// Stream a CAN log's filtered frames to `writer` as CSV (`wide=true`) or JSONL.
/// Reuses `can_parse`'s per-chunk `DocLines` reader + `build_frame_source`, so it
/// does NOT hold the doc lock for the whole run (concurrent `doc_chunk` reads
/// interleave). `frame_json` decodes each frame once; `frame_matches` decides
/// inclusion. Source-build or write failure sets `errored` and stops (no `Result`);
/// `cancel` is checked per frame.
#[allow(dead_code)] // consumed by the export commands (Task 5)
#[allow(clippy::too_many_arguments)] // scan inputs (doc/dbc/source/filter/sink/callbacks)
pub fn export_can<W: Write>(
    doc_arc: &Arc<Mutex<DocFile>>,
    dbc: Option<&Database>,
    kind: &str,
    regex: &Option<String>,
    filter: &CanFilterJson,
    writer: &mut W,
    on_progress: &mut dyn FnMut(u64),
    cancel: &dyn Fn() -> bool,
    wide: bool,
) -> CanExportStats {
    let lines = DocLines::new(doc_arc.clone());
    let mut src = match build_frame_source(kind, regex, lines) {
        Ok(s) => s,
        Err(_) => {
            return CanExportStats {
                written: 0,
                scanned: 0,
                cancelled: false,
                errored: true,
            }
        }
    };
    let cols = db_signal_columns(dbc);
    let col_index: HashMap<String, usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| (c.clone(), CSV_META.len() + i))
        .collect();
    let ncols = CSV_META.len() + cols.len();
    if wide && writer.write_all(csv_header(dbc).as_bytes()).is_err() {
        return CanExportStats {
            written: 0,
            scanned: 0,
            cancelled: false,
            errored: true,
        };
    }
    let (mut written, mut scanned) = (0u64, 0u64);
    loop {
        if cancel() {
            on_progress(written);
            return CanExportStats {
                written,
                scanned,
                cancelled: true,
                errored: false,
            };
        }
        let Some(f) = src.next_frame() else { break };
        scanned += 1;
        let fj = frame_json(&f, dbc);
        if !frame_matches(&fj, filter) {
            continue;
        }
        let row = if wide {
            csv_row(&fj, &col_index, ncols)
        } else {
            jsonl_line(&fj)
        };
        if writer.write_all(row.as_bytes()).is_err() {
            return CanExportStats {
                written,
                scanned,
                cancelled: false,
                errored: true,
            };
        }
        written += 1;
        if scanned % CAN_EXPORT_BATCH == 0 {
            on_progress(written);
        }
    }
    on_progress(written);
    CanExportStats {
        written,
        scanned,
        cancelled: false,
        errored: false,
    }
}

/// True iff `fj` satisfies every active axis of `f` (AND), matching
/// `canfilter.ts`'s `applyCanFilter` exactly — including `Object.is` float
/// equality for `==` (bit-equal: `NaN==NaN` is true, `+0 != -0`).
pub fn frame_matches(fj: &FrameJson, f: &CanFilterJson) -> bool {
    // ID closed range; a reversed range matches nothing.
    if let Some(from) = f.id_from {
        if fj.can_id < from {
            return false;
        }
    }
    if let Some(to) = f.id_to {
        if fj.can_id > to {
            return false;
        }
    }
    if let (Some(from), Some(to)) = (f.id_from, f.id_to) {
        if from > to {
            return false;
        }
    }
    // Message name exact (frames without a msg name are dropped when the axis is set).
    if let Some(name) = &f.msg_name {
        if fj.msg_name.as_deref() != Some(name.as_str()) {
            return false;
        }
    }
    // Signal condition: active only when name + op + value are all present.
    if let (Some(sig), Some(op), Some(val)) = (&f.signal_name, f.signal_op, f.signal_value) {
        let physical = match fj.signals.iter().find(|s| &s.name == sig) {
            Some(s) => s.physical,
            None => return false, // no such signal -> excluded (Object.is-safe)
        };
        let ok = match op {
            SignalOp::Gt => physical > val,
            SignalOp::Lt => physical < val,
            SignalOp::Ge => physical >= val,
            SignalOp::Le => physical <= val,
            SignalOp::Eq => physical.to_bits() == val.to_bits(),
        };
        if !ok {
            return false;
        }
    }
    // Time window: if either bound is set, frames without a timestamp drop out.
    if f.time_from_ms.is_some() || f.time_to_ms.is_some() {
        let ts = match fj.timestamp_ms {
            Some(t) => t,
            None => return false,
        };
        if let Some(from) = f.time_from_ms {
            if ts < from {
                return false;
            }
        }
        if let Some(to) = f.time_to_ms {
            if ts > to {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::SignalValueJson;

    fn sig(name: &str, physical: f64) -> SignalValueJson {
        SignalValueJson {
            name: name.to_string(),
            raw: physical as i64,
            physical,
            enum_text: None,
            unit: String::new(),
        }
    }
    fn frame(can_id: u32, msg_name: Option<&str>, ts: Option<f64>, signals: Vec<SignalValueJson>) -> FrameJson {
        FrameJson {
            line_number: 1,
            timestamp_ms: ts,
            channel: 0,
            can_id,
            can_id_hex: format!("0x{can_id:X}"),
            dlc: 8,
            data_hex: String::new(),
            is_remote: false,
            is_fd: false,
            error_frame: false,
            msg_name: msg_name.map(|s| s.to_string()),
            signals,
        }
    }
    fn f() -> CanFilterJson {
        CanFilterJson::default()
    }

    #[test]
    fn empty_filter_keeps_all() {
        assert!(frame_matches(&frame(0x100, None, None, vec![]), &f()));
    }

    #[test]
    fn id_closed_range() {
        let mut flt = f();
        flt.id_from = Some(0x100);
        flt.id_to = Some(0x1ff);
        assert!(frame_matches(&frame(0x100, None, None, vec![]), &flt));
        assert!(frame_matches(&frame(0x1ff, None, None, vec![]), &flt));
        assert!(!frame_matches(&frame(0x200, None, None, vec![]), &flt));
    }

    #[test]
    fn id_reversed_matches_none() {
        let mut flt = f();
        flt.id_from = Some(0x200);
        flt.id_to = Some(0x100);
        assert!(!frame_matches(&frame(0x180, None, None, vec![]), &flt));
    }

    #[test]
    fn msg_name_exact() {
        let mut flt = f();
        flt.msg_name = Some("A".into());
        assert!(frame_matches(&frame(1, Some("A"), None, vec![]), &flt));
        assert!(!frame_matches(&frame(1, Some("B"), None, vec![]), &flt));
        assert!(!frame_matches(&frame(1, None, None, vec![]), &flt), "no msg dropped when set");
    }

    #[test]
    fn signal_gt_and_missing() {
        let mut flt = f();
        flt.signal_name = Some("Speed".into());
        flt.signal_op = Some(SignalOp::Gt);
        flt.signal_value = Some(100.0);
        assert!(frame_matches(&frame(1, Some("M"), None, vec![sig("Speed", 500.0)]), &flt));
        assert!(!frame_matches(&frame(1, Some("M"), None, vec![sig("Speed", 50.0)]), &flt));
        assert!(!frame_matches(&frame(1, Some("M"), None, vec![sig("Other", 500.0)]), &flt), "missing signal excludes");
    }

    #[test]
    fn signal_eq_object_is_semantics() {
        let mut flt = f();
        flt.signal_name = Some("S".into());
        flt.signal_op = Some(SignalOp::Eq);
        // NaN == NaN under Object.is (bit-equal), unlike f64 ==.
        flt.signal_value = Some(f64::NAN);
        assert!(frame_matches(&frame(1, Some("M"), None, vec![sig("S", f64::NAN)]), &flt), "Object.is(NaN,NaN) true");
        flt.signal_value = Some(5.0);
        assert!(frame_matches(&frame(1, Some("M"), None, vec![sig("S", 5.0)]), &flt));
        assert!(!frame_matches(&frame(1, Some("M"), None, vec![sig("S", 5.0001)]), &flt));
    }

    #[test]
    fn time_window_no_ts_drops() {
        let mut flt = f();
        flt.time_from_ms = Some(0.0);
        flt.time_to_ms = Some(1000.0);
        assert!(frame_matches(&frame(1, None, Some(500.0), vec![]), &flt));
        assert!(!frame_matches(&frame(1, None, Some(2000.0), vec![]), &flt));
        assert!(!frame_matches(&frame(1, None, None, vec![]), &flt), "ts-less dropped when window active");
    }

    #[test]
    fn axes_combine_with_and() {
        let mut flt = f();
        flt.id_from = Some(0x100);
        flt.id_to = Some(0x1ff);
        flt.msg_name = Some("EngineData".into());
        flt.signal_name = Some("Speed".into());
        flt.signal_op = Some(SignalOp::Ge);
        flt.signal_value = Some(60.0);
        assert!(frame_matches(&frame(0x123, Some("EngineData"), Some(10.0), vec![sig("Speed", 60.0)]), &flt));
        assert!(!frame_matches(&frame(0x999, Some("EngineData"), Some(10.0), vec![sig("Speed", 99.0)]), &flt), "id out");
    }

    // ---- text builders (Task 3) ----
    use lumelog_can::dbc::{ByteOrder, Message, Mux, Signal};

    fn dbc_sig(name: &str) -> Signal {
        Signal {
            name: name.to_string(),
            start_bit: 0,
            length: 8,
            byte_order: ByteOrder::LittleEndian,
            is_signed: false,
            factor: 1.0,
            offset: 0.0,
            minimum: 0.0,
            maximum: 0.0,
            unit: String::new(),
            receivers: vec![],
            value_table: vec![],
            mux: Mux::NonMuxed,
        }
    }
    fn dbc_msg(id: u32, name: &str, sigs: &[&str]) -> Message {
        Message {
            id,
            name: name.to_string(),
            dlc: 8,
            transmitter: String::new(),
            is_extended: false,
            signals: sigs.iter().map(|s| dbc_sig(s)).collect(),
        }
    }
    fn sample_db() -> Database {
        Database {
            version: String::new(),
            nodes: vec![],
            messages: vec![
                dbc_msg(1, "A", &["Speed", "Temp"]),
                dbc_msg(2, "B", &["Speed"]),
            ],
        }
    }

    #[test]
    fn qualified_columns_basic_and_dedup() {
        let cols = qualified_columns(&[("A".into(), vec!["S".into(), "S".into()])]);
        assert_eq!(cols, vec!["A.S", "A.S#2"]);
    }

    #[test]
    fn db_signal_columns_qualified_order() {
        assert_eq!(db_signal_columns(Some(&sample_db())), vec!["A.Speed", "A.Temp", "B.Speed"]);
        assert!(db_signal_columns(None).is_empty());
    }

    #[test]
    fn csv_header_meta_only_without_db() {
        assert_eq!(
            csv_header(None),
            "lineNumber,timestampMs,channel,canId,msgName,dlc,dataHex\r\n"
        );
    }

    #[test]
    fn csv_header_includes_signal_columns() {
        let h = csv_header(Some(&sample_db()));
        assert!(h.ends_with(",A.Speed,A.Temp,B.Speed\r\n"), "{h}");
    }

    #[test]
    fn csv_row_places_signals_and_blanks_others() {
        let sig_cols = db_signal_columns(Some(&sample_db()));
        let mut index = HashMap::new();
        for (i, name) in sig_cols.iter().enumerate() {
            index.insert(name.clone(), CSV_META.len() + i);
        }
        let ncols = CSV_META.len() + sig_cols.len();
        let fj = frame(1, Some("A"), Some(1000.0), vec![sig("Speed", 500.0), sig("Temp", 90.0)]);
        assert_eq!(csv_row(&fj, &index, ncols), "1,1000,0,1,A,8,,500,90,\r\n");
    }

    #[test]
    fn csv_row_escapes_and_handles_no_msg() {
        let index = HashMap::new();
        let ncols = CSV_META.len();
        let mut fj = frame(3, None, None, vec![]);
        fj.data_hex = "a,b".into(); // comma triggers quoting
        // `frame()` helper pins lineNumber=1, channel=0, dlc=8; canId=3, msgName empty.
        assert_eq!(csv_row(&fj, &index, ncols), "1,,0,3,,8,\"a,b\"\r\n");
    }

    #[test]
    fn fmt_physical_stable_and_nonfinite() {
        assert_eq!(fmt_physical(512.0), "512");
        assert_eq!(fmt_physical(50.25), "50.25");
        assert_eq!(fmt_physical(0.1 + 0.2), "0.3"); // {:.6} then trim
        assert_eq!(fmt_physical(f64::NAN), "");
        assert_eq!(fmt_physical(f64::INFINITY), "");
    }

    #[test]
    fn jsonl_line_is_valid_json_with_signals() {
        let fj = frame(5, Some("A"), Some(1.5), vec![sig("Speed", 42.0)]);
        let line = jsonl_line(&fj);
        assert!(line.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(parsed["canId"].as_u64(), Some(5));
        assert_eq!(parsed["msgName"].as_str(), Some("A"));
        assert_eq!(parsed["signals"][0]["physical"].as_f64(), Some(42.0));
    }

    // ---- export_can (Task 4) ----
    use tempfile::NamedTempFile;

    fn doc_arc(text: &str) -> (NamedTempFile, Arc<Mutex<DocFile>>) {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(text.as_bytes()).unwrap();
        f.flush().unwrap();
        let d = DocFile::open(f.path(), 1 << 20).unwrap();
        (f, Arc::new(Mutex::new(d)))
    }
    const CANDUMP: &str = "can0 100#DEADBEEF\ncan0 101#CAFEBEEF\n";

    #[test]
    fn export_jsonl_writes_valid_frame_lines() {
        let (_k, arc) = doc_arc(CANDUMP);
        let mut out: Vec<u8> = Vec::new();
        let s = export_can(&arc, None, "candump", &None, &f(), &mut out, &mut |_| {}, &|| false, false);
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(s.written, 2);
        assert_eq!(s.scanned, 2);
        let p0: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(p0["canId"].as_u64(), Some(0x100));
        assert!(!s.errored && !s.cancelled);
    }

    #[test]
    fn export_csv_header_and_id_filter() {
        let (_k, arc) = doc_arc(CANDUMP);
        let mut flt = f();
        flt.id_from = Some(0x101);
        flt.id_to = Some(0x101);
        let mut out: Vec<u8> = Vec::new();
        let s = export_can(&arc, None, "candump", &None, &flt, &mut out, &mut |_| {}, &|| false, true);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("lineNumber,timestampMs,channel,canId,msgName,dlc,dataHex\r\n"));
        // only the 0x101 (257) frame survives the id filter
        assert_eq!(s.written, 1);
        assert_eq!(s.scanned, 2);
        assert!(text.contains(",257,"), "{text}");
    }

    #[test]
    fn export_csv_includes_dbc_signal_columns() {
        let (_k, arc) = doc_arc("can0 1#AABBCCDD\n"); // id 1 -> message A(id 1)
        let mut db = sample_db(); // A(1){Speed,Temp}, B(2){Speed}
        db.messages[0].id = 1;
        let mut out: Vec<u8> = Vec::new();
        let s = export_can(&arc, Some(&db), "candump", &None, &f(), &mut out, &mut |_| {}, &|| false, true);
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("lineNumber,timestampMs,channel,canId,msgName,dlc,dataHex,A.Speed,A.Temp,B.Speed\r\n"), "{text}");
        assert_eq!(s.written, 1);
    }

    #[test]
    fn export_cancel_stops_and_is_prefix() {
        let big = "can0 100#AA\n".repeat(20);
        let (_k, arc) = doc_arc(&big);
        let seen = std::cell::Cell::new(0u32);
        let mut out: Vec<u8> = Vec::new();
        let s = export_can(
            &arc,
            None,
            "candump",
            &None,
            &f(),
            &mut out,
            &mut |_| {},
            &|| {
                let c = seen.get();
                seen.set(c + 1);
                c >= 3
            },
            false,
        );
        assert!(s.cancelled, "cancel reported");
        assert!((s.written as usize) < 20);
    }
}
