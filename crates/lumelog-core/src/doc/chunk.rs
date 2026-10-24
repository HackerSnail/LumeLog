//! A decoded chunk of consecutive lines plus its byte<->(line, cu16) map.
//!
//! `chunk_at` hands the frontend a block of display text for a line range, and
//! a [`CharMap`] that converts an absolute file byte offset (e.g. a search hit)
//! into an absolute display line plus a UTF-16 code-unit offset within that
//! line — the coordinate space CodeMirror uses (spec §4.2, §6.3).

use crate::doc::line_index::LineIndex;
use crate::encoding::{self, decode_all, decode_region, encode_all, EncodingLabel};

/// One decoded logical line inside a chunk.
///
/// A logical line normally corresponds to one indexed (raw-byte) line, but the
/// GB18030 mis-split retry (spec §4.1) can merge two or more raw lines into a
/// single logical line, so we track the raw byte span as well.
#[derive(Debug, Clone)]
pub struct LogicalLine {
    /// Absolute byte offset of the first raw byte of this line's source span.
    pub raw_start: u64,
    /// Absolute byte offset one past the last raw byte (includes the trailing
    /// `0x0A`, and the span of any raw lines merged into this logical line).
    pub raw_end: u64,
    /// Absolute byte offset where `text`'s first decoded character begins.
    /// Equals `raw_start` except for the file's first line, where a leading
    /// BOM was stripped by the decoder (`raw_start + bom_len`).
    pub text_start: u64,
    /// Decoded text of the line, including its trailing `\n` if present.
    pub text: String,
}

/// Byte-offset <-> (line, cu16-within-line) map for a chunk.
///
/// v1 stores one [`LogicalLine`] per display line and walks its characters on
/// demand (a line is short, so a query is O(line length)). ASCII-run sampling
/// compression from spec §4.2 is deferred as a later optimization; correctness
/// of the split/alignment rules is pinned first.
#[derive(Debug, Clone)]
pub struct CharMap {
    label: EncodingLabel,
    start_line: u32,
    lines: Vec<LogicalLine>,
}

impl CharMap {
    /// Build from already-split logical lines (the path `build_chunk` uses, so
    /// merges and BOM offsets are fully under the caller's control).
    pub fn from_lines(start_line: u32, label: EncodingLabel, lines: Vec<LogicalLine>) -> CharMap {
        CharMap {
            label,
            start_line,
            lines,
        }
    }

    /// Convenience for a plain, BOM-free, merge-free chunk: split `text` on
    /// `\n` (keeping the delimiter) and align each piece to a consecutive
    /// indexed line starting at `start_line`.
    pub fn from_text(
        start_line: u32,
        text: &str,
        index: &LineIndex,
        label: EncodingLabel,
    ) -> CharMap {
        let mut lines = Vec::new();
        for (line_no, seg) in (start_line as usize..).zip(split_keepends(text)) {
            let (rs, re) = index.line_range(line_no).unwrap_or((0, 0));
            lines.push(LogicalLine {
                raw_start: rs,
                raw_end: re,
                text_start: rs,
                text: seg.to_string(),
            });
        }
        CharMap::from_lines(start_line, label, lines)
    }

    /// Number of logical display lines in this map (== indexed lines unless a
    /// merge-retry fired).
    pub fn logical_line_count(&self) -> usize {
        self.lines.len()
    }

    /// Convert an absolute file byte offset into `(absolute line, cu16 in
    /// line)`. A byte landing at a character boundary yields that character's
    /// starting cu16 offset; a byte inside the stripped BOM yields cu16 0.
    /// Bytes past the last line are clamped to the last line's end.
    pub fn byte_to_line_cu(&self, abs_byte: u64) -> (usize, u32) {
        if self.lines.is_empty() {
            return (self.start_line as usize, 0);
        }
        for (i, l) in self.lines.iter().enumerate() {
            if abs_byte < l.raw_end || i + 1 == self.lines.len() {
                let line = self.start_line as usize + i;
                if abs_byte <= l.text_start {
                    return (line, 0);
                }
                let mut pos = l.text_start;
                let mut cu = 0u32;
                for c in l.text.chars() {
                    let sl = src_len(self.label, c) as u64;
                    if abs_byte < pos + sl {
                        break; // abs_byte falls inside (or at the start of) c
                    }
                    pos += sl;
                    cu += c.len_utf16() as u32;
                }
                return (line, cu);
            }
        }
        unreachable!()
    }
}

/// A decoded block of consecutive lines.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub start_line: usize,
    pub text: String,
    pub invalid_count: usize,
    pub char_map: CharMap,
}

/// Approximate heap footprint of a chunk, used to bound the DocFile LRU.
pub fn chunk_size(chunk: &Chunk) -> usize {
    chunk.text.len()
        + chunk
            .char_map
            .lines
            .iter()
            .map(|l| l.text.len() + std::mem::size_of::<LogicalLine>())
            .sum::<usize>()
}

/// Source byte length a single character occupied in `label`.
///
/// UTF variants use the character's own width; legacy codecs re-encode the
/// character (valid round-trip text reproduces the exact source width).
fn src_len(label: EncodingLabel, c: char) -> usize {
    match label {
        EncodingLabel::Utf8 | EncodingLabel::Utf8Bom => c.len_utf8(),
        EncodingLabel::Utf16Le | EncodingLabel::Utf16Be => c.len_utf16() * 2,
        other => encode_all(other, c.encode_utf8(&mut [0u8; 4])).bytes.len(),
    }
}

fn split_keepends(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out.push(&text[start..i + 1]);
            start = i + 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    // A chunk of zero lines still yields one (possibly empty) logical line.
    if out.is_empty() {
        out.push("");
    }
    out
}

fn ends_with_replacement(text: &str) -> bool {
    matches!(text.chars().last(), Some('\u{FFFD}'))
}

/// Decode lines `[start_line, start_line + count)` of `bytes`.
///
/// `bytes` are the raw file bytes for the requested line range, i.e.
/// `[index.line_range(start_line).start .. index.line_range(end).end]`, and
/// `abs_start` is that first absolute offset. Applies the GB18030 mis-split
/// retry: a raw line whose decode ends in a U+FFFD (a truncated multi-byte
/// sequence cut at the `0x0A` boundary) is merged with the following raw line
/// and re-decoded, spec §4.1 / §5.4.
pub fn build_chunk(
    bytes: &[u8],
    abs_start: u64,
    index: &LineIndex,
    label: EncodingLabel,
    start_line: usize,
    count: usize,
) -> Chunk {
    let end_line = (start_line + count).min(index.line_count());
    // Contract: `bytes` must cover every indexed line in `[start_line, end_line)`,
    // i.e. `abs` lands in `[abs_start, abs_start + bytes.len()]`. `chunk_at` upholds
    // this by rejecting short reads before calling; the `debug_assert` catches any
    // future caller that violates it, and `.min` stays only as a release safety net
    // so a violation degrades to a clamped slice instead of aborting a user's edit.
    let to_rel = |abs: u64| {
        debug_assert!(
            abs >= abs_start && abs <= abs_start + bytes.len() as u64,
            "byte {abs} outside buffer [{abs_start}, {})",
            abs_start + bytes.len() as u64
        );
        (abs - abs_start).min(bytes.len() as u64) as usize
    };

    let mut lines: Vec<LogicalLine> = Vec::new();
    let mut text = String::new();
    let mut invalid_total = 0usize;

    let mut li = start_line;
    while li < end_line {
        let (ls, le) = index.line_range(li).unwrap_or((abs_start, abs_start));
        let mut seg = &bytes[to_rel(ls)..to_rel(le)];
        // Only the file's first line (span starting at offset 0) may carry a
        // BOM to strip; every other span is decoded region-wise so an interior
        // BOM-looking byte sequence stays as content (I-2).
        let mut dec = if ls == 0 {
            decode_all(label, seg)
        } else {
            decode_region(label, seg)
        };
        let mut raw_end = le;

        // Defensive merge-retry for a truncated multi-byte tail. NOTE: for the
        // nine supported codecs `0x0A` is never a continuation byte, so a
        // non-final raw line always ends in a real `'\n'` and this branch is
        // currently unreachable; it is kept so a future codec whose LF can
        // appear as a trail byte is handled without code churn (spec §4.1).
        while dec.invalid_count > 0
            && ends_with_replacement(&dec.text)
            && li + 1 < end_line
        {
            li += 1;
            let (_, le2) = index.line_range(li).unwrap_or((le, le));
            raw_end = le2;
            seg = &bytes[to_rel(ls)..to_rel(le2)];
            dec = if ls == 0 {
                decode_all(label, seg)
            } else {
                decode_region(label, seg)
            };
        }

        // The file's first logical line may have had its BOM stripped.
        let bom_skip = if ls == 0 {
            encoding::leading_bom_len(label, seg)
        } else {
            0
        };
        text.push_str(&dec.text);
        invalid_total += dec.invalid_count;
        lines.push(LogicalLine {
            raw_start: ls,
            raw_end,
            text_start: ls + bom_skip as u64,
            text: dec.text,
        });
        li += 1;
    }

    Chunk {
        start_line,
        text,
        invalid_count: invalid_total,
        char_map: CharMap::from_lines(start_line as u32, label, lines),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::line_index::build;

    #[test]
    fn chunk_first_two_lines_utf8() {
        let data = b"alpha\nbeta\ngamma\n";
        let idx = build(data).unwrap();
        let c = build_chunk(data, 0, &idx, EncodingLabel::Utf8, 0, 2);
        assert_eq!(c.start_line, 0);
        assert_eq!(c.text, "alpha\nbeta\n");
        assert_eq!(c.invalid_count, 0);
        assert_eq!(c.char_map.byte_to_line_cu(0), (0, 0));
        assert_eq!(c.char_map.byte_to_line_cu(6), (1, 0)); // 'b' of beta
        assert_eq!(c.char_map.byte_to_line_cu(8), (1, 2)); // 't' in beta
    }

    #[test]
    fn valid_multiline_gbk_never_misplits() {
        // Grounding the ruling: `0x0A` is never a GBK continuation byte, so a
        // valid GBK stream's raw byte-line count equals its true line count and
        // every line round-trips with zero replacements. 3 newlines => 4
        // segments (last empty), exactly as the byte scanner should produce.
        let text = "台架HIL测试\n数据记录完成\n尾行\n";
        let bytes = encode_all(EncodingLabel::Gbk, text).bytes;
        let idx = build(&bytes).unwrap();
        assert_eq!(idx.line_count(), 4, "3 newlines + trailing empty line");
        let c = build_chunk(&bytes, 0, &idx, EncodingLabel::Gbk, 0, idx.line_count());
        assert_eq!(c.text, text);
        assert_eq!(c.invalid_count, 0);
    }

    #[test]
    fn charmap_maps_astral_and_multibyte_boundaries() {
        // line 0 = "a" U+1F600 "\n": a=1 byte/1 cu, emoji=4 bytes/2 cu, \n=1/1.
        let text = "a\u{1F600}\nb\n";
        let bytes = text.as_bytes().to_vec();
        let idx = build(&bytes).unwrap();
        let c = build_chunk(&bytes, 0, &idx, EncodingLabel::Utf8, 0, 2);
        assert_eq!(c.char_map.byte_to_line_cu(0), (0, 0)); // 'a'
        assert_eq!(c.char_map.byte_to_line_cu(1), (0, 1)); // start of emoji
        assert_eq!(c.char_map.byte_to_line_cu(5), (0, 3)); // '\n' (cu 1+2)
        assert_eq!(c.char_map.byte_to_line_cu(6), (1, 0)); // 'b' on line 1
    }

    #[test]
    fn charmap_from_text_matches_from_lines_no_bom() {
        // The convenience builder (split on '\n', align to index lines) must
        // agree with the general path for a plain, merge-free, BOM-free chunk.
        let text = "one\ntwo\nthree\n";
        let bytes = text.as_bytes();
        let idx = build(bytes).unwrap();
        let via_text = CharMap::from_text(0, text, &idx, EncodingLabel::Utf8);
        let via_lines = build_chunk(bytes, 0, &idx, EncodingLabel::Utf8, 0, idx.line_count());
        for b in 0..bytes.len() as u64 {
            assert_eq!(
                via_text.byte_to_line_cu(b),
                via_lines.char_map.byte_to_line_cu(b),
                "byte {b}"
            );
        }
    }

    #[test]
    fn malformed_dangling_lead_byte_does_not_panic() {
        // A lone GBK lead byte before a real newline: invalid, but chunking must
        // index it and surface U+FFFD without panicking. Because a non-final raw
        // line always terminates in a real '\n', the merge-retry branch does NOT
        // fire (logical lines == indexed lines) — this pins that behavior.
        let bytes = [0xCA_u8, 0x0A, b'x', 0x0A];
        let idx = build(&bytes).unwrap();
        let c = build_chunk(&bytes, 0, &idx, EncodingLabel::Gbk, 0, idx.line_count());
        assert!(c.invalid_count >= 1, "dangling lead byte yields a replacement");
        assert_eq!(
            c.char_map.logical_line_count(),
            idx.line_count(),
            "merge-retry must not fire (0x0A is a real newline)"
        );
        // byte_to_line_cu must terminate for every position in range.
        for b in 0..bytes.len() as u64 {
            let _ = c.char_map.byte_to_line_cu(b);
        }
    }
}
