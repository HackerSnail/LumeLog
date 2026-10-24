//! Tests for the generic regex source (spec §7.3.3 / Review Focus 5).

use lumelog_can::error::CanError;
use lumelog_can::frame::{Frame, SourceStats};
use lumelog_can::source::{FrameSource, Line, RegexSource};

type Src = RegexSource<std::vec::IntoIter<Line>>;

fn src(pattern: &str, text: &str) -> Src {
    let lines: Vec<Line> =
        text.lines().enumerate().map(|(i, l)| ((i + 1) as u64, l.to_string())).collect();
    RegexSource::new(lines.into_iter(), pattern).expect("pattern should build")
}

fn drain(mut s: Src) -> (Vec<Frame>, SourceStats) {
    let mut v = Vec::new();
    while let Some(f) = s.next_frame() {
        v.push(f);
    }
    (v, s.stats())
}

#[test]
fn builds_from_pattern_with_named_groups() {
    // id + data required, ts optional — all present here.
    let p = r"ts=(?P<ts>[\d.]+) id=(?P<id>[0-9A-Fa-f]+) data=(?P<data>[0-9A-Fa-f ]+)";
    let s = src(p, "ts=1.5 id=1AB data=DEADBEEF");
    let (frames, _) = drain(s);
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].timestamp, Some(1.5));
    assert_eq!(frames[0].can_id, 0x1AB);
}

#[test]
fn missing_required_group_errors() {
    // No `data` group → BadRegex at build time (Review Focus 5).
    let p = r"id=(?P<id>[0-9A-Fa-f]+)";
    let lines: Vec<Line> = vec![(1, "id=1AB".to_string())];
    let err = RegexSource::new(lines.into_iter(), p).unwrap_err();
    assert!(matches!(err, CanError::BadRegex(_)), "got {err:?}");
    assert_eq!(err.ipc_code(), "bad_regex");
    // And a missing `id` group likewise.
    let p2 = r"data=(?P<data>[0-9A-Fa-f]+)";
    let err2 = RegexSource::new(vec![(1, String::new())].into_iter(), p2).unwrap_err();
    assert!(matches!(err2, CanError::BadRegex(_)));
}

#[test]
fn invalid_regex_compile_errors() {
    let lines: Vec<Line> = vec![(1, String::new())];
    let err = RegexSource::new(lines.into_iter(), r"id=(?P<id)[") .unwrap_err();
    assert!(matches!(err, CanError::RegexCompile(_)), "got {err:?}");
}

#[test]
fn decodes_custom_log_lines() {
    // A self-invented dialect with 0x-prefixed id and colon-separated data.
    let p = r"^ID=(?P<id>\S+) PAYLOAD=(?P<data>[\w:]+)$";
    let (frames, st) = drain(src(p, "ID=0x735 PAYLOAD=11:22:33:44"));
    assert_eq!(frames.len(), 1);
    assert_eq!(st.decoded, 1);
    assert_eq!(st.skipped, 0);
    let f = &frames[0];
    assert_eq!(f.can_id, 0x735, "0x prefix tolerated");
    assert_eq!(f.dlc, 4);
    assert_eq!(&f.data[..4], &[0x11, 0x22, 0x33, 0x44]);
    assert_eq!(f.timestamp, None, "no ts group → None");
}

#[test]
fn non_matching_line_skipped_counted() {
    let p = r"id=(?P<id>[0-9A-Fa-f]+) data=(?P<data>[0-9A-Fa-f]+)";
    let text = "id=123 data=AA\nnothing here\nid=456 data=BB";
    let (frames, st) = drain(src(p, text));
    assert_eq!(frames.len(), 2);
    assert_eq!(st.decoded, 2);
    assert_eq!(st.skipped, 1, "middle line counted");
}

#[test]
fn data_length_drives_dlc() {
    // dlc = hex char count / 2, regardless of declared length.
    let p = r"id=(?P<id>[0-9A-Fa-f]+) data=(?P<data>[0-9A-Fa-f]+)";
    let (frames, _) = drain(src(p, "id=1 data=112233"));
    assert_eq!(frames[0].dlc, 3);
    let (frames, _) = drain(src(p, "id=1 data=AABB"));
    assert_eq!(frames[0].dlc, 2);
}

#[test]
fn odd_or_invalid_hex_data_is_skipped() {
    // Odd-length / non-hex payload → the line is skipped, not a frame.
    let p = r"id=(?P<id>[0-9A-Fa-f]+) data=(?P<data>\S+)";
    let (frames, st) = drain(src(p, "id=1 data=ABC\nid=1 data=ZZ"));
    assert!(frames.is_empty());
    assert_eq!(st.decoded, 0);
    assert_eq!(st.skipped, 2);
}
