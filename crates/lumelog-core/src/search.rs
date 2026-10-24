//! Streaming search over a [`DocFile`] (spec §6.2 / A8, Review Focus 4).
//!
//! The haystack is the file's *raw bytes*, scanned in fixed windows so a
//! multi-gigabyte log never has to be held in memory. The query string is first
//! encoded into the file's own encoding (`A8`: "查询串按此编码 encode"), then
//! matched against those raw bytes:
//!
//! * **Literal** — the encoded query bytes are hex-escaped into a byte regex, so
//!   a Chinese query finds the GBK byte sequence actually stored on disk (not its
//!   UTF-8 spelling).
//! * **Regex** — the pattern is used as a `regex::bytes` pattern. This matches
//!   byte-for-byte for UTF-8 files and for ASCII patterns in any codec (a very
//!   common log need). Non-ASCII regex patterns over a legacy codec are not
//!   meaningful — use Literal.
//!
//! Cross-window matches are handled by re-reading an `OVERLAP`-byte tail between
//! consecutive windows. Each match is emitted exactly once by confining emission
//! to a window's *committed* region `[start, start + step)`; the overlap tail is
//! left to the next window, which re-reads it in full. This requires a match to
//! be no longer than `OVERLAP` (a documented v1 bound: log queries are short).
//!
//! UTF-16 search is deferred to Plan 3 (its interleaved NUL bytes make a
//! byte-space scan meaningless), so a query whose encoded form contains `0x00`
//! returns [`CoreError::Encoding`].

use regex::bytes::{Regex, RegexBuilder};

use crate::doc::DocFile;
use crate::encoding::{self, EncodingLabel};
use crate::error::CoreError;

/// Scanning window size (8 MiB, spec §6.2).
const WINDOW: usize = 8 * 1024 * 1024;
/// Tail re-read between windows; also the upper bound on a single match length.
const OVERLAP: usize = 1024;
/// Decoded characters of context kept on each side of a hit in the snippet.
const SNIPPET_CU: u32 = 40;

/// How to interpret [`SearchQuery::pattern_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Literal,
    Regex,
}

/// A compiled-or-to-be-compiled search request.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub pattern_text: String,
    pub mode: Mode,
    /// Encoding the pattern is encoded into before matching raw file bytes
    /// (spec §6.2 / A8). Usually the document's own detected encoding.
    pub encoding: EncodingLabel,
    pub case_sensitive: bool,
}

/// One hit. `line`/`byte_in_line` are in raw-byte space (line-number stability
/// survives encoding switches, spec §5.2); the frontend turns `byte_in_line`
/// into CodeMirror columns via the chunk [`CharMap`](crate::doc::CharMap).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// 0-based line index containing the start of the hit.
    pub line: u32,
    /// Byte offset of the hit start within that line.
    pub byte_in_line: u64,
    /// Byte length of the matched span in the raw file.
    pub len: usize,
    /// Decoded text of the containing line, trimmed to ±40 code units around the
    /// hit (newlines stripped).
    pub snippet: String,
}

/// Aggregate result of a scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchStats {
    pub matches: u64,
    pub scanned_bytes: u64,
    pub cancelled: bool,
}

/// Search the whole file. `progress` is called once per window with the number
/// of bytes scanned so far and returns `false` to cancel; `on_match` receives
/// every hit in ascending file order.
pub fn search_stream(
    doc: &mut DocFile,
    q: &SearchQuery,
    mut on_match: impl FnMut(Match),
    mut progress: impl FnMut(u64) -> bool,
) -> Result<SearchStats, CoreError> {
    search_with_window(doc, q, &mut on_match, &mut progress, WINDOW, OVERLAP)
}

/// Testable core: same as [`search_stream`] but with the window/overlap sizes
/// injected, so seam behavior can be pinned on tiny buffers without an 8 MiB
/// file. Production callers use [`search_stream`].
fn search_with_window(
    doc: &mut DocFile,
    q: &SearchQuery,
    on_match: &mut impl FnMut(Match),
    progress: &mut impl FnMut(u64) -> bool,
    window: usize,
    overlap: usize,
) -> Result<SearchStats, CoreError> {
    let re = build_regex(q)?;
    // Each window advances by (window - overlap); the trailing `overlap` bytes
    // are re-read by the next window so a match straddling the seam is seen.
    let step = (window - overlap) as u64;
    let mut cursor: u64 = 0;
    let mut stats = SearchStats {
        matches: 0,
        scanned_bytes: 0,
        cancelled: false,
    };

    loop {
        let win_start = cursor;
        let win = doc.read_window(win_start, window)?;
        let win_len = win.len() as u64;
        let is_final = (win_len as usize) < window;
        // Matches starting in [win_start, committed_end) belong to this window
        // and are fully contained in it; the tail [committed_end, win_start +
        // win_len) is the next window's committed start, so skipping it here
        // prevents a straddling match from being emitted twice.
        let committed_end = if is_final {
            win_start + win_len
        } else {
            win_start + step
        };

        for m in re.find_iter(&win) {
            let abs = win_start + m.start() as u64;
            if abs >= committed_end {
                break; // later matches start even further into the tail
            }
            let len = m.end() - m.start();
            let line = doc.line_at_byte(abs) as u32;
            let line_start = doc
                .line_byte_range(line as usize)
                .map(|(s, _)| s)
                .unwrap_or(abs);
            let byte_in_line = abs - line_start;
            let snippet = make_snippet(doc, line as usize, abs, len)?;
            on_match(Match {
                line,
                byte_in_line,
                len,
                snippet,
            });
            stats.matches += 1;
        }

        stats.scanned_bytes = win_start + win_len;
        if is_final || !progress(stats.scanned_bytes) {
            stats.cancelled = !is_final;
            break;
        }
        cursor = win_start + step;
    }

    Ok(stats)
}

/// Compile the raw-byte regex from the query, rejecting UTF-16 and empty
/// literal patterns up front.
fn build_regex(q: &SearchQuery) -> Result<Regex, CoreError> {
    // `A8`: encode the query into the file's codec, then match raw bytes. A
    // `0x00` in the result means UTF-16, which we refuse (see module docs).
    let qbytes = encoding::encode_all(q.encoding, &q.pattern_text).bytes;
    if qbytes.contains(&0u8) {
        return Err(CoreError::Encoding(
            "UTF-16 search unsupported in v1".to_string(),
        ));
    }

    let pattern = match q.mode {
        Mode::Literal => {
            if qbytes.is_empty() {
                return Err(CoreError::Encoding("empty search pattern".to_string()));
            }
            // Hex-escape every byte so regex metacharacters (and arbitrary
            // GBK byte values) are matched literally against the raw file.
            let mut s = String::with_capacity(qbytes.len() * 4);
            for b in &qbytes {
                s.push_str(&format!("\\x{:02X}", b));
            }
            s
        }
        Mode::Regex => q.pattern_text.clone(),
    };

    let mut builder = RegexBuilder::new(&pattern);
    builder.case_insensitive(!q.case_sensitive);
    if matches!(q.mode, Mode::Literal) {
        // In Unicode mode `\xC4` means the scalar U+00C4 (matched as its UTF-8
        // bytes); turning Unicode off makes each `\xHH` the single raw byte HH,
        // which is what a byte-literal scan of a legacy-encoded file needs.
        builder.unicode(false);
    }
    builder
        .build()
        .map_err(|e| CoreError::Encoding(e.to_string()))
}

/// Decode the hit's line and keep the ±40 code-unit window around the match,
/// stripping the trailing newline(s).
fn make_snippet(
    doc: &mut DocFile,
    line: usize,
    abs: u64,
    mlen: usize,
) -> Result<String, CoreError> {
    // One-line chunk hands back the decoded text and the byte->(line, cu) map.
    let chunk = doc.chunk_at(line, 1)?;
    let (_, cu_start) = chunk.char_map.byte_to_line_cu(abs);
    let (_, cu_end) = chunk.char_map.byte_to_line_cu(abs + mlen as u64);
    let lo = cu_start.saturating_sub(SNIPPET_CU);
    let hi = cu_end + SNIPPET_CU;

    let mut out = String::new();
    let mut cu = 0u32;
    for c in chunk.text.chars() {
        if c == '\n' || c == '\r' {
            continue;
        }
        if cu >= hi {
            break;
        }
        if cu >= lo {
            out.push(c);
        }
        cu += c.len_utf16() as u32;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::encode_all;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    fn temp_log(name: &str, bytes: &[u8]) -> PathBuf {
        let mut dir = std::env::temp_dir();
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        dir.push(format!("lumelog_search_{}_{name}_{id}.log", std::process::id()));
        let mut f = std::fs::File::create(&dir).unwrap();
        f.write_all(bytes).unwrap();
        dir
    }

    fn q(pattern_text: &str, mode: Mode, encoding: EncodingLabel, case: bool) -> SearchQuery {
        SearchQuery {
            pattern_text: pattern_text.to_string(),
            mode,
            encoding,
            case_sensitive: case,
        }
    }

    // Collect hits without snippets-heavy checks.
    fn collect(doc: &mut DocFile, query: &SearchQuery) -> (Vec<Match>, SearchStats) {
        let mut hits = Vec::new();
        let stats = search_stream(doc, query, |m| hits.push(m), |_| true).unwrap();
        (hits, stats)
    }

    #[test]
    fn literal_hits_gbk_chinese() {
        // "错误" appears twice in a GBK log; the query is the same word encoded
        // to GBK, so a byte-space scan must find exactly those two spots.
        let text = "动力系统 错误 记录\n正常 心跳\n再次的 错误 结束\n";
        let bytes = encode_all(EncodingLabel::Gbk, text).bytes;
        let p = temp_log("gbk", &bytes);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let (hits, stats) = collect(&mut doc, &q("错误", Mode::Literal, EncodingLabel::Gbk, true));
        assert_eq!(hits.len(), 2, "two GBK 错误 hits");
        assert_eq!(stats.matches, 2);
        assert_eq!(stats.scanned_bytes, bytes.len() as u64);
        assert_eq!(hits[0].line, 0);
        assert_eq!(hits[1].line, 2);
        // snippet decodes back to the Chinese context, not mojibake.
        assert!(hits[0].snippet.contains("错误"), "{:?}", hits[0].snippet);
        fs::remove_file(&p).ok();
    }

    #[test]
    fn regex_overlap_match_not_missed_not_doubled() {
        // Tiny window/overlap forces a seam. A literal "ABCD" straddles the
        // boundary between one window's committed region and the next window's
        // overlap; it must be found exactly once.
        // window 8, overlap 4 -> step 4: committed regions [0,4),[4,8),...
        // Place "ABCD" at byte 2 so it spans [2,6): partly committed, partly tail.
        let data = b"xxABCDyyzzABCDww";
        let p = temp_log("seam", data);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let mut hits = Vec::new();
        let stats = search_with_window(
            &mut doc,
            &q("ABCD", Mode::Literal, EncodingLabel::Utf8, true),
            &mut |m| hits.push(m),
            &mut |_| true,
            8,
            4,
        )
        .unwrap();
        let starts: Vec<u64> = hits.iter().map(|h| h.byte_in_line).collect();
        assert_eq!(stats.matches, 2, "both ABCD occurrences found once each");
        assert_eq!(starts, vec![2, 10], "no doubling, no miss: {starts:?}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn cancel_stops_early() {
        // Two windows (window 8, step 4 over a 16-byte file => 4 windows).
        // Cancel after the 2nd progress call; matches must be fewer than a full
        // scan reports, and cancelled is set.
        let data = b"AAxxAAxxAAxxAAxx";
        let p = temp_log("cancel", data);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();

        let mut calls = 0u32;
        let mut hits = Vec::new();
        let stats = search_with_window(
            &mut doc,
            &q("AA", Mode::Literal, EncodingLabel::Utf8, true),
            &mut |m| hits.push(m),
            &mut |_| {
                calls += 1;
                calls < 2 // false on the 2nd call cancels
            },
            8,
            4,
        )
        .unwrap();
        assert!(stats.cancelled, "cancel must be reported");
        // A full scan finds 6 "AA"?? (overlapping not counted; find_iter is
        // non-overlapping: positions 0,8 -> within 16 bytes with step windows).
        // Regardless of the exact full count, cancelling mid-way finds fewer.
        let full = {
            let mut d2 = DocFile::open(&p, 1 << 20).unwrap();
            let (h, _) = collect(&mut d2, &q("AA", Mode::Literal, EncodingLabel::Utf8, true));
            h.len()
        };
        assert!(
            stats.matches < full as u64,
            "cancelled {} should be < full {full}",
            stats.matches
        );
        fs::remove_file(&p).ok();
    }

    #[test]
    fn line_number_correct() {
        // "needle" is on the 3rd line (index 2); hit line must report 2 and the
        // byte_in_line must be its offset within that line.
        let data = b"one\ntwo\nxxx needle yyy\nfour\n";
        let p = temp_log("lines", data);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let (hits, _) = collect(&mut doc, &q("needle", Mode::Literal, EncodingLabel::Utf8, true));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 2, "0-based line index");
        // line 2 starts at byte 8 ("one\ntwo\n"); "xxx " is 4 bytes -> offset 4
        assert_eq!(hits[0].byte_in_line, 4);
        fs::remove_file(&p).ok();
    }

    #[test]
    fn stats_counts_all_matches_gb18030_file() {
        // Every "ERROR" in an ASCII-in-GB18030 file is counted; multi-hit line.
        let text = "ERROR a\nb ERROR c ERROR\nd\n";
        let bytes = encode_all(EncodingLabel::Gb18030, text).bytes;
        let p = temp_log("gb18030", &bytes);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let (hits, stats) = collect(&mut doc, &q("ERROR", Mode::Literal, EncodingLabel::Gb18030, true));
        assert_eq!(stats.matches, 3);
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[1].line, 1);
        assert_eq!(hits[2].line, 1, "two ERRORs share line 1");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn utf16_query_is_rejected() {
        let p = temp_log("u16", b"whatever");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let err = search_stream(
            &mut doc,
            &q("abc", Mode::Literal, EncodingLabel::Utf16Le, true),
            |_| {},
            |_| true,
        )
        .unwrap_err();
        assert!(
            matches!(err, CoreError::Encoding(ref m) if m.contains("UTF-16")),
            "got {err:?}"
        );
        fs::remove_file(&p).ok();
    }

    #[test]
    fn case_insensitive_finds_both_cases() {
        let data = b"Error error ERROR\n";
        let p = temp_log("ci", data);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let (hits, _) = collect(
            &mut doc,
            &q("error", Mode::Literal, EncodingLabel::Utf8, false),
        );
        assert_eq!(hits.len(), 3, "case-insensitive literal matches all forms");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn regex_mode_alternation_over_utf8() {
        let data = b"log error warn error fatal\n";
        let p = temp_log("regex", data);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let (hits, _) = collect(
            &mut doc,
            &q("err(or|no)", Mode::Regex, EncodingLabel::Utf8, true),
        );
        assert_eq!(hits.len(), 2);
        fs::remove_file(&p).ok();
    }
}
