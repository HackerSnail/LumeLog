//! Selected-line export (spec §7.5 `export_lines`, M3 increment 2). Writes the
//! raw source bytes of the lines a [`LineFilter`] keeps, so the output preserves
//! the file's original encoding and line endings exactly (no decode/re-encode).
//!
//! It lives in core because the raw byte view (`line_byte_range`/`read_window`)
//! is `pub(crate)`; the `lumelog-app` command opens the target file and passes a
//! `Write` here. Read/write errors are absorbed into [`ExportStats::errored`] so
//! the command always reports a final tally (the caller flushes the writer).

use std::io::Write;

use crate::doc::DocFile;
use crate::filter::LineFilter;

/// Decode lines of `text` (a `chunk_at` block), yielding them WITHOUT their
/// trailing terminator — byte-for-byte what `filter_run`'s `DocLines` feeds to
/// the matcher, so keep-decisions match the on-screen filtered view.
pub(crate) fn chunk_lines(text: &str) -> Vec<&str> {
    let mut body = text;
    if let Some(b) = body.strip_suffix("\r\n") {
        body = b;
    } else if let Some(b) = body.strip_suffix('\n') {
        body = b;
    }
    body.split('\n')
        .map(|raw| raw.strip_suffix('\r').unwrap_or(raw))
        .collect()
}

/// Tally of one export run. `errored` folds any read/write failure (disk full,
/// file shrank) after the bytes already written are kept; `cancelled` = stopped
/// early on user request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportStats {
    pub written: u64,
    pub scanned: u64,
    pub cancelled: bool,
    pub errored: bool,
}

const EXPORT_BATCH: usize = 4096;

/// Scan the whole file and write the raw bytes of every line `matcher` keeps to
/// `writer`. `on_progress(written)` fires once per block; `cancel` is checked per
/// line so a huge export stops promptly. Never returns an error — failures set
/// `errored` and stop, preserving what was already written.
pub fn run<W: Write>(
    doc: &mut DocFile,
    matcher: &LineFilter,
    writer: &mut W,
    on_progress: &mut dyn FnMut(u64),
    cancel: &dyn Fn() -> bool,
) -> ExportStats {
    let total = doc.line_count();
    let (mut written, mut scanned) = (0u64, 0u64);
    let (mut cancelled, mut errored) = (false, false);
    let mut start = 0usize;

    'outer: while start < total {
        let count = EXPORT_BATCH.min(total - start);
        let chunk = match doc.chunk_at(start, count) {
            Ok(c) => c,
            Err(_) => {
                errored = true;
                break;
            }
        };
        let lines = chunk_lines(&chunk.text);
        for (i, text) in lines.iter().enumerate() {
            let line_no = start + i;
            if cancel() {
                cancelled = true;
                break 'outer;
            }
            scanned += 1;
            if matcher.keep(line_no as u64, text).is_none() {
                continue;
            }
            // Raw bytes (terminator included) preserve the source exactly.
            let (s, e) = match doc.line_byte_range(line_no) {
                Some(r) => r,
                None => {
                    errored = true;
                    break 'outer;
                }
            };
            match doc.read_window(s, (e - s) as usize) {
                Ok(bytes) => {
                    if writer.write_all(&bytes).is_err() {
                        errored = true;
                        break 'outer;
                    }
                    written += 1;
                }
                Err(_) => {
                    errored = true;
                    break 'outer;
                }
            }
        }
        on_progress(written);
        start += count;
    }

    ExportStats { written, scanned, cancelled, errored }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{self, EncodingLabel};
    use std::path::PathBuf;

    fn temp_file(bytes: &[u8]) -> PathBuf {
        let mut dir = std::env::temp_dir();
        static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let id = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        dir.push(format!("lumelog_export_{}_{id}.log", std::process::id()));
        let mut f = std::fs::File::create(&dir).unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        dir
    }

    fn keep_all() -> LineFilter {
        // No conditions/timeline/range → every line kept (empty ranges).
        LineFilter::build(vec![], true, None, None).unwrap()
    }

    #[test]
    fn export_selected_lines_byte_parity() {
        let p = temp_file(b"a\nbb\nccc\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let lf =
            LineFilter::build(vec![crate::filter::CondSpec {
                mode: crate::filter::CondMode::Include,
                kind: crate::filter::CondKind::Literal,
                text: "b".to_string(),
            }], true, None, None)
            .unwrap();
        let mut out: Vec<u8> = Vec::new();
        let stats = run(&mut doc, &lf, &mut out, &mut |_| {}, &|| false);
        assert_eq!(out, b"bb\n", "only the line containing 'b', raw bytes with terminator");
        assert_eq!((stats.written, stats.scanned), (1, 3));
        assert!(!stats.cancelled && !stats.errored);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn export_all_preserves_exact_bytes_crlf_no_trailing_newline() {
        let src = b"one\r\ntwo\r\nthree"; // CRLF, last line has no terminator
        let p = temp_file(src);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let mut out: Vec<u8> = Vec::new();
        let stats = run(&mut doc, &keep_all(), &mut out, &mut |_| {}, &|| false);
        assert_eq!(out, src.to_vec(), "byte-for-byte identical, incl. CRLF + missing final NL");
        assert_eq!(stats.written, 3);
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn export_preserves_multibyte_bytes() {
        let bytes = encoding::encode_all(EncodingLabel::Gbk, "错误 日志\nok\n").bytes;
        let p = temp_file(&bytes);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let mut out: Vec<u8> = Vec::new();
        let _ = run(&mut doc, &keep_all(), &mut out, &mut |_| {}, &|| false);
        assert_eq!(out, bytes, "GBK bytes copied verbatim (no transcoding)");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn export_cancel_stops_early() {
        let mut src = Vec::new();
        for i in 0..20 {
            src.extend_from_slice(format!("line {i}\n").as_bytes());
        }
        let p = temp_file(&src);
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let seen = std::cell::Cell::new(0u32);
        let mut out: Vec<u8> = Vec::new();
        let stats = run(&mut doc, &keep_all(), &mut out, &mut |_| {}, &|| {
            let c = seen.get();
            seen.set(c + 1);
            c >= 5
        });
        assert!(stats.cancelled, "cancel reported");
        assert!((stats.written as usize) < 20, "stopped before all lines");
        // output is a prefix of the source
        assert!(src.starts_with(&out), "written bytes are a source prefix");
        std::fs::remove_file(&p).ok();
    }
}
