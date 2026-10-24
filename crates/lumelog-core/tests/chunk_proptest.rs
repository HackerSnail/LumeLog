//! Proptest: `CharMap::byte_to_line_cu` must agree with an independent oracle
//! built straight from `decode_all` of byte prefixes (Review Focus 1 / 5).
//!
//! For every byte offset the returned **line** must equal `LineIndex::line_at_byte`
//! (lines are byte-based, encoding-independent). For UTF-8 (BOM or not) we also
//! pin the **cu16 offset** at every character boundary against the UTF-16 code
//! units of `decode_all(bytes[line_start..byte])`. The corpus is round-trip
//! valid text (encode(decode)==identity), so `decode_all` on a prefix is a
//! genuinely different code path from the per-character walk in `CharMap`.

use lumelog_core::doc::{build, build_aligned, build_chunk, LineIndex};
use lumelog_core::encoding::{decode_all, encode_all, EncodingLabel};
use proptest::prelude::*;

/// Build the index the way `DocFile::open` does: aligned 16-bit scan for
/// UTF-16, raw `0x0A` scan otherwise.
fn index_for(label: EncodingLabel, bytes: &[u8]) -> LineIndex {
    match label {
        EncodingLabel::Utf16Le => build_aligned(bytes, 0x000A, true).unwrap(),
        EncodingLabel::Utf16Be => build_aligned(bytes, 0x000A, false).unwrap(),
        _ => build(bytes).unwrap(),
    }
}

fn corpus() -> Vec<(EncodingLabel, Vec<u8>)> {
    let rich = "2026-10-24 12:00:01 INFO 台架HIL测试\nCAN 0x123 数据😀记录\n\n尾行\n";
    let cjk = "动力系统控制台\n台架测试数据记录完成\n中文尾行\n";
    let mut v: Vec<(EncodingLabel, Vec<u8>)> = Vec::new();
    for t in [rich, "a\nbb\nccc\n", "\n\n\n", "no trailing newline"] {
        v.push((EncodingLabel::Utf8, t.as_bytes().to_vec()));
        v.push((EncodingLabel::Utf8Bom, encode_all(EncodingLabel::Utf8Bom, t).bytes));
    }
    for t in [cjk, "line1\nline2\n"] {
        v.push((EncodingLabel::Gbk, encode_all(EncodingLabel::Gbk, t).bytes));
        v.push((EncodingLabel::Gb18030, encode_all(EncodingLabel::Gb18030, t).bytes));
        v.push((EncodingLabel::Utf16Le, encode_all(EncodingLabel::Utf16Le, t).bytes));
    }
    v
}

/// Oracle: (line, cu16) for a byte offset, derived only from `decode_all` and
/// the byte-based `LineIndex`.
fn reference(label: EncodingLabel, bytes: &[u8], idx: &LineIndex, byte: u64) -> (usize, u32) {
    let line = idx.line_at_byte(byte);
    let (ls, _) = idx.line_range(line).unwrap();
    let prefix = &bytes[ls as usize..byte as usize];
    let dec = decode_all(label, prefix);
    (line, dec.text.encode_utf16().count() as u32)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn charmap_all_alignments((label, bytes) in prop::sample::select(corpus())) {
        let idx = index_for(label, &bytes);
        let whole = decode_all(label, &bytes);
        let c = build_chunk(&bytes, 0, &idx, label, 0, idx.line_count());

        // (A) chunk text equals a whole-buffer decode.
        prop_assert_eq!(&c.text, &whole.text);

        // (C) line resolution matches the byte-based index at every offset.
        for b in 0..bytes.len() as u64 {
            let (line, _cu) = c.char_map.byte_to_line_cu(b);
            prop_assert_eq!(line, idx.line_at_byte(b), "line at byte {}", b);
        }

        // (B) for UTF-8, pin cu16 at every character boundary.
        if matches!(label, EncodingLabel::Utf8 | EncodingLabel::Utf8Bom) {
            for b in 0..bytes.len() {
                let is_boundary = b == 0 || (bytes[b] & 0xC0) != 0x80;
                if is_boundary {
                    let got = c.char_map.byte_to_line_cu(b as u64);
                    let exp = reference(label, &bytes, &idx, b as u64);
                    prop_assert_eq!(got, exp, "cu mismatch at byte {}", b);
                }
            }
        }
    }
}
