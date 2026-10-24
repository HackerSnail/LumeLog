//! Tests for the Vector ASC source (spec §7.3.2 / Review Focus 3).

use lumelog_can::frame::{Frame, SourceStats};
use lumelog_can::source::{AscSource, FrameSource, Line};

fn src(text: &str) -> AscSource<std::vec::IntoIter<Line>> {
    let lines: Vec<Line> =
        text.lines().enumerate().map(|(i, l)| ((i + 1) as u64, l.to_string())).collect();
    AscSource::new(lines.into_iter())
}

fn drain(mut s: AscSource<std::vec::IntoIter<Line>>) -> (Vec<Frame>, SourceStats) {
    let mut v = Vec::new();
    while let Some(f) = s.next_frame() {
        v.push(f);
    }
    (v, s.stats())
}

#[test]
fn parses_data_frames_with_channel_and_id() {
    let (frames, st) = drain(src("1.000000 2 123 Rx d 8 DE AD BE EF 00 11 22 33"));
    assert_eq!(frames.len(), 1);
    assert_eq!(st.decoded, 1);
    assert_eq!(st.skipped, 0);
    let f = &frames[0];
    assert_eq!(f.line_number, 1);
    assert_eq!(f.timestamp, Some(1.0));
    assert_eq!(f.channel, 2);
    assert_eq!(f.can_id, 0x123);
    assert_eq!(f.dlc, 8);
    assert_eq!(&f.data[..8], &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33]);
    assert!(!f.flags.is_remote);
}

#[test]
fn extended_id_x_suffix() {
    let (frames, _) = drain(src("2.500000 1 1A7x Rx d 4 11 22 33 44"));
    assert_eq!(frames[0].can_id, 0x1A7);
    assert_eq!(frames[0].timestamp, Some(2.5));
    assert_eq!(frames[0].dlc, 4);
}

#[test]
fn remote_frame_rx_r() {
    let (frames, _) = drain(src("3.000000 1 321 Tx r 3"));
    assert!(frames[0].flags.is_remote);
    assert_eq!(frames[0].dlc, 3);
    assert!(frames[0].data.iter().all(|&b| b == 0));
}

#[test]
fn skips_boe_bcs_error_counted() {
    // boe/bcs lines carry a real timestamp+Rx but an unsupported frame type → skipped.
    let text = "4.000000 1 100 Rx boe 1 2\n5.000000 1 100 Rx bcs 8";
    let (frames, st) = drain(src(text));
    assert!(frames.is_empty());
    assert_eq!(st.decoded, 0);
    assert_eq!(st.skipped, 2);
}

#[test]
fn malformed_line_does_not_stop_stream() {
    let text = "date Mon Jul 01 10:00:00 2024\nnot a frame\n1.000000 1 123 Rx d 1 FF\nbase 0";
    let (frames, st) = drain(src(text));
    // Only the middle line is a frame; the rest are skipped and counted.
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].can_id, 0x123);
    assert_eq!(st.decoded, 1);
    assert_eq!(st.skipped, 3, "lines 1,2,4 counted");
}

#[test]
fn fixture_file_decodes_expected_counts() {
    let text = include_str!("data/asc_basic.asc");
    let (frames, st) = drain(src(text));
    // 3 valid frames (data / extended / remote); 4 skip lines (date, boe, base, CAN-prefixed).
    assert_eq!(frames.len(), 3);
    assert_eq!(st.decoded, 3);
    assert_eq!(st.skipped, 4);
}
