//! Proptest: incremental decode must be split-invariant (Review Focus 1).
//! Splitting the byte stream at any points and concatenating `feed` output
//! (plus the final `finish` flush) must equal `decode_all`; summed
//! `invalid_count` must match too. The corpus deliberately includes BOM'd
//! UTF-16/UTF-8, a GBK stream truncated mid-character (exercises the flush
//! path), a valid UTF-8 stream carrying literal U+FFFD, and arbitrary random
//! bytes (invalid in every codec) so the property is not vacuous.

use lumelog_core::encoding::{decode_all, encode_all, EncodingLabel, IncrementalDecoder};
use proptest::prelude::*;

fn corpus() -> Vec<(EncodingLabel, Vec<u8>)> {
    let text = "2026-10-24 12:00:01 INFO 台架HIL测试\nCAN 0x123 数据 记录 完成\n尾行\n";
    let mut cases = vec![
        (
            EncodingLabel::Gbk,
            encode_all(EncodingLabel::Gbk, text).bytes,
        ),
        (
            EncodingLabel::Gb18030,
            encode_all(EncodingLabel::Gb18030, text).bytes,
        ),
        (EncodingLabel::Utf8, text.as_bytes().to_vec()),
        (
            EncodingLabel::Big5,
            encode_all(EncodingLabel::Big5, text).bytes,
        ),
        (
            EncodingLabel::ShiftJis,
            encode_all(EncodingLabel::ShiftJis, text).bytes,
        ),
        // BOM variants: incremental must strip the BOM across split boundaries
        // exactly like decode_all.
        (
            EncodingLabel::Utf16Le,
            encode_all(EncodingLabel::Utf16Le, text).bytes,
        ),
        (
            EncodingLabel::Utf16Be,
            encode_all(EncodingLabel::Utf16Be, text).bytes,
        ),
        (
            EncodingLabel::Utf8Bom,
            encode_all(EncodingLabel::Utf8Bom, text).bytes,
        ),
    ];
    // Truncated multi-byte tail: drop GBK trail byte -> flush yields U+FFFD.
    let mut gbk_trunc = encode_all(EncodingLabel::Gbk, "数据线记录").bytes;
    gbk_trunc.pop();
    cases.push((EncodingLabel::Gbk, gbk_trunc));
    // Truncated UTF-16 unit (odd trailing byte) -> flush must match decode_all.
    let mut u16_trunc = encode_all(EncodingLabel::Utf16Le, "ab线").bytes;
    u16_trunc.push(b'x');
    cases.push((EncodingLabel::Utf16Le, u16_trunc));
    // Literal U+FFFD inside valid UTF-8: replacement counting must stay
    // consistent between the two code paths.
    cases.push((
        EncodingLabel::Utf8,
        "bad\u{FFFD}token\u{FFFD} ok\n".as_bytes().to_vec(),
    ));
    cases
}

fn label_pool() -> Vec<EncodingLabel> {
    vec![
        EncodingLabel::Gbk,
        EncodingLabel::Gb18030,
        EncodingLabel::Utf8,
        EncodingLabel::Utf8Bom,
        EncodingLabel::Big5,
        EncodingLabel::ShiftJis,
        EncodingLabel::Utf16Le,
        EncodingLabel::Utf16Be,
    ]
}

fn arbitrary_input() -> impl Strategy<Value = (EncodingLabel, Vec<u8>)> {
    let corpus_case = prop::sample::select(corpus());
    let random_case = (
        prop::sample::select(label_pool()),
        prop::collection::vec(any::<u8>(), 0..512),
    );
    prop_oneof![corpus_case, random_case]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn incremental_split_anywhere_matches_whole(
        (label, bytes) in arbitrary_input(),
        cuts in prop::collection::vec(0usize..(1usize << 16), 1..6),
    ) {
        let whole = decode_all(label, &bytes);

        let mut dec = IncrementalDecoder::new(label).unwrap();
        let mut text = String::new();
        let mut invalid = 0usize;

        // Split points include 0 and len; dedup keeps them distinct, but the
        // modulo can also collapse onto a neighbour, so zero-length chunks
        // (empty `feed`) are exercised too.
        let mut pts: Vec<usize> = cuts.iter().map(|&c| c % (bytes.len() + 1)).collect();
        pts.push(0);
        pts.push(bytes.len());
        pts.sort_unstable();

        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let seg = dec.feed(&bytes[a..b]);
            text.push_str(&seg.text);
            invalid += seg.invalid_count;
        }
        let tail = dec.finish();
        text.push_str(&tail.text);
        invalid += tail.invalid_count;

        prop_assert_eq!(text, whole.text);
        prop_assert_eq!(invalid, whole.invalid_count);
    }
}
