//! Proptest: incremental decode must be split-invariant (Review Focus 1).
//! Splitting the byte stream at any points and concatenating `feed` output
//! must equal `decode_all`; summed `invalid_count` must match too.

use lumelog_core::encoding::{decode_all, encode_all, EncodingLabel, IncrementalDecoder};
use proptest::prelude::*;

fn corpus() -> Vec<(EncodingLabel, Vec<u8>)> {
    let text = "2026-10-24 12:00:01 INFO 台架HIL测试\nCAN 0x123 数据 记录 完成\n尾行\n";
    vec![
        (EncodingLabel::Gbk, encode_all(EncodingLabel::Gbk, text).bytes),
        (
            EncodingLabel::Gb18030,
            encode_all(EncodingLabel::Gb18030, text).bytes,
        ),
        (EncodingLabel::Utf8, encode_all(EncodingLabel::Utf8, text).bytes),
        (EncodingLabel::Big5, encode_all(EncodingLabel::Big5, text).bytes),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn incremental_split_anywhere_matches_whole(
        (label, bytes) in prop::sample::select(corpus()),
        cuts in prop::collection::vec(0usize..(1usize << 16), 1..4),
    ) {
        let whole = decode_all(label, &bytes);

        let mut dec = IncrementalDecoder::new(label).unwrap();
        let mut text = String::new();
        let mut invalid = 0usize;

        let mut pts: Vec<usize> = cuts.iter().map(|&c| c % (bytes.len() + 1)).collect();
        pts.push(0);
        pts.push(bytes.len());
        pts.sort_unstable();
        pts.dedup();

        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let seg = dec.feed(&bytes[a..b]);
            text.push_str(&seg.text);
            invalid += seg.invalid_count;
        }
        invalid += dec.finish();

        prop_assert_eq!(text, whole.text);
        prop_assert_eq!(invalid, whole.invalid_count);
    }
}
