//! Golden tests for the DBC subset parser (spec §7.1). `sample.dbc` deliberately
//! contains one malformed `SG_` line (line 14) to pin the strict/lenient split.

use lumelog_can::dbc::{parse, parse_lenient, ByteOrder, Database, Message, Mux, Signal};
use lumelog_can::error::CanError;

const SAMPLE: &str = include_str!("data/sample.dbc");

fn msg<'a>(db: &'a Database, name: &str) -> &'a Message {
    db.messages.iter().find(|m| m.name == name).expect("message")
}
fn sig<'a>(m: &'a Message, name: &str) -> &'a Signal {
    m.signals.iter().find(|s| s.name == name).expect("signal")
}

#[test]
fn parses_sample_messages_and_signals() {
    let db = parse_lenient(SAMPLE).database;
    assert_eq!(db.messages.len(), 3);
    let eng = msg(&db, "EngineData");
    assert_eq!(eng.id, 291);
    assert!(!eng.is_extended);
    assert_eq!(eng.dlc, 8);
    assert_eq!(eng.transmitter, "ECU1");
    // BrokenSignal (line 14) is skipped → only two good signals survive.
    assert_eq!(eng.signals.len(), 2);
    let speed = sig(eng, "EngineSpeed");
    assert_eq!(speed.byte_order, ByteOrder::LittleEndian);
    assert!(!speed.is_signed);
    assert_eq!(speed.start_bit, 0);
    assert_eq!(speed.length, 16);
    assert_eq!(speed.factor, 0.125);
    assert_eq!(speed.offset, 0.0);
    assert_eq!(speed.receivers, vec!["Dashboard".to_string()]);
}

#[test]
fn big_endian_signed_signal_exact() {
    let db = parse_lenient(SAMPLE).database;
    let ext = msg(&db, "ExtFrame");
    assert!(ext.is_extended, "raw id had bit31 set");
    assert_eq!(ext.id, 0x500, "id is the 29-bit masked value");
    let a = sig(ext, "SignalA");
    assert_eq!(a.byte_order, ByteOrder::BigEndian);
    assert!(a.is_signed);
    assert_eq!(a.start_bit, 7);
    assert_eq!(a.length, 16);
    assert_eq!(a.factor, 0.5);
    assert_eq!(a.offset, -10.0);
}

#[test]
fn skips_comments_and_attributes() {
    // CM_/BA_DEF_/BA_ must not fabricate a message or add a signal.
    let db = parse_lenient(SAMPLE).database;
    assert_eq!(db.messages.iter().filter(|m| m.name == "EngineData").count(), 1);
    assert_eq!(sig(msg(&db, "EngineData"), "EngineSpeed").length, 16);
}

#[test]
fn val_table_populates_enum() {
    let db = parse_lenient(SAMPLE).database;
    let temp = sig(msg(&db, "EngineData"), "CoolantTemp");
    assert_eq!(temp.value_table.len(), 3);
    assert!(temp.value_table.contains(&(90, "Normal".to_string())));
}

#[test]
fn mux_marker_parsed() {
    let db = parse_lenient(SAMPLE).database;
    let muxed = msg(&db, "MuxedMsg");
    assert_eq!(sig(muxed, "Mode").mux, Mux::Selector);
    assert_eq!(sig(muxed, "Speed").mux, Mux::Indexed(0));
    assert_eq!(sig(muxed, "Temp").mux, Mux::Indexed(1));
    assert_eq!(sig(msg(&db, "EngineData"), "EngineSpeed").mux, Mux::NonMuxed);
}

#[test]
fn lenient_skips_bad_line_records_context() {
    let outcome = parse_lenient(SAMPLE);
    assert_eq!(outcome.errors.len(), 1, "exactly one bad line, got {outcome:?}");
    match &outcome.errors[0] {
        CanError::DbcSyntax { line, kind, .. } => {
            assert_eq!(*line, 14);
            assert_eq!(kind, "bad SG_");
        }
        other => panic!("expected DbcSyntax, got {other:?}"),
    }
    // lenient still yields a usable database
    assert_eq!(outcome.database.messages.len(), 3);

    // strict mode aborts on that same line
    match parse(SAMPLE) {
        Err(CanError::DbcSyntax { line, .. }) => assert_eq!(line, 14),
        other => panic!("expected Err(DbcSyntax) at line 14, got {other:?}"),
    }
}

#[test]
fn signal_length_out_of_range_is_rejected() {
    // A regex-matching SG_ with length 65 (> one 64-bit frame) must be rejected
    // so decode can never shift a u64 by >= 64.
    let src = "BO_ 100 M: 8 N\n SG_ Big : 0|65@1+ (1,0) [0|1] \"\" R\n";
    match parse(src) {
        Err(CanError::DbcSyntax { line, kind, .. }) => {
            assert_eq!(line, 2);
            assert_eq!(kind, "signal length out of range");
        }
        other => panic!("expected DbcSyntax, got {other:?}"),
    }
    let outcome = parse_lenient(src);
    assert_eq!(outcome.errors.len(), 1, "{outcome:?}");
    assert!(outcome.database.messages[0].signals.is_empty());
}
