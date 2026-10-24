//! Tests for the candump source (spec §7.3.1 / Review Focus 3).

use lumelog_can::frame::{Frame, SourceStats};
use lumelog_can::source::{CandumpSource, FrameSource, Line};

fn src(text: &str) -> CandumpSource<std::vec::IntoIter<Line>> {
    let lines: Vec<Line> =
        text.lines().enumerate().map(|(i, l)| ((i + 1) as u64, l.to_string())).collect();
    CandumpSource::new(lines.into_iter())
}

fn drain(mut s: CandumpSource<std::vec::IntoIter<Line>>) -> (Vec<Frame>, SourceStats) {
    let mut v = Vec::new();
    while let Some(f) = s.next_frame() {
        v.push(f);
    }
    (v, s.stats())
}

#[test]
fn parses_plain_and_ts_frames() {
    let (frames, st) = drain(src("(1.234567) can0 123#DEADBEEF"));
    assert_eq!(frames.len(), 1);
    assert_eq!(st.decoded, 1);
    assert_eq!(st.skipped, 0);
    let f = &frames[0];
    assert_eq!(f.line_number, 1);
    assert_eq!(f.timestamp, Some(1.234567));
    assert_eq!(f.can_id, 0x123);
    assert_eq!(f.dlc, 4);
    assert_eq!(&f.data[..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
    assert!(!f.flags.is_fd);
}

#[test]
fn no_timestamp_is_none() {
    let (frames, _) = drain(src("can0 735#1122"));
    assert_eq!(frames[0].timestamp, None);
    assert_eq!(frames[0].can_id, 0x735);
}

#[test]
fn channel_from_iface() {
    let (frames, _) = drain(src("can2 1F#A1B2"));
    assert_eq!(frames[0].channel, 2);
    let (frames, _) = drain(src("vcan11 1F#A1B2"));
    assert_eq!(frames[0].channel, 11);
}

#[test]
fn rtr_sets_remote_no_data() {
    let (frames, _) = drain(src("can0 123#R"));
    assert!(frames[0].flags.is_remote);
    assert_eq!(frames[0].dlc, 0);
    assert_eq!(frames[0].can_id, 0x123);
}

#[test]
fn fd_frame_flags_and_long_data() {
    let (frames, _) = drain(src("can0 123##031122334455667788"));
    let f = &frames[0];
    assert!(f.flags.is_fd);
    assert!(f.flags.brs, "flags byte 0x03 has bit0");
    assert!(f.flags.esi, "flags byte 0x03 has bit1");
    assert_eq!(f.dlc, 8);
    assert_eq!(&f.data[..8], &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
}

#[test]
fn extended_id_8_hex() {
    let (frames, _) = drain(src("can0 1FFFFFFF#42"));
    assert_eq!(frames[0].can_id, 0x1FFF_FFFF);
}

#[test]
fn garbage_and_error_are_skipped_counted() {
    let text = "can0 123#DEADBEEF\ncan0 123#NOThex\ngarbage line\n\ncan0 broken";
    let (frames, st) = drain(src(text));
    assert_eq!(frames.len(), 1);
    assert_eq!(st.decoded, 1);
    assert_eq!(st.skipped, 3, "lines 2,3,5 counted; blank line 4 not");
}

#[test]
fn fixture_file_decodes_expected_counts() {
    let text = include_str!("data/candump_basic.log");
    let (frames, st) = drain(src(text));
    // 7 valid frames (incl RTR, FD, extended); 2 skip lines (garbage + NOThex).
    assert_eq!(frames.len(), 7);
    assert_eq!(st.decoded, 7);
    assert_eq!(st.skipped, 2);
}
