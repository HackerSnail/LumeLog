//! Integration tests for the `DocFile` open/chunk path over real temp files.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use lumelog_core::doc::DocFile;
use lumelog_core::encoding::{encode_all, EncodingLabel};
use lumelog_core::error::CoreError;

fn write_tmp(bytes: &[u8]) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut p = std::env::temp_dir();
    p.push(format!("lumelog_test_{}_{id}.log", std::process::id()));
    let mut f = fs::File::create(&p).unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
    p
}

#[test]
fn open_and_chunk_utf8() {
    let p = write_tmp(b"a\nb\nc\n");
    let mut doc = DocFile::open(&p, 1 << 20).unwrap();
    assert_eq!(doc.encoding(), EncodingLabel::Utf8);
    assert_eq!(doc.line_count(), 4, "3 newlines + trailing empty line");
    let c = doc.chunk_at(0, 2).unwrap();
    assert_eq!(c.text, "a\nb\n");
    fs::remove_file(&p).ok();
}

#[test]
fn chunk_at_zero_lines_returns_empty_not_panic() {
    // C-1: a zero-height viewport must yield an empty chunk, not underflow.
    let p = write_tmp(b"a\nb\nc\n");
    let mut doc = DocFile::open(&p, 1 << 20).unwrap();
    let c = doc.chunk_at(0, 0).unwrap();
    assert_eq!(c.text, "");
    assert_eq!(c.start_line, 0);
    fs::remove_file(&p).ok();
}

#[test]
fn interior_bom_is_preserved_as_content() {
    // I-2: only the file's leading BOM is stripped; an interior EF BB BF is
    // content (U+FEFF), and its line's char_map stays aligned.
    let bytes = [0xEF, 0xBB, 0xBF, b'a', 0x0A, 0xEF, 0xBB, 0xBF, b'b', 0x0A];
    let p = write_tmp(&bytes);
    let mut doc = DocFile::open(&p, 1 << 20).unwrap();
    assert_eq!(doc.encoding(), EncodingLabel::Utf8Bom);
    assert_eq!(doc.line_count(), 3, "2 newlines + trailing empty line");
    assert_eq!(doc.chunk_at(0, 1).unwrap().text, "a\n", "leading BOM stripped");
    let line1 = doc.chunk_at(1, 1).unwrap();
    assert_eq!(line1.text, "\u{FEFF}b\n", "interior BOM kept as content");
    // char_map stays aligned: byte 5 is the interior BOM's first byte (cu 0),
    // byte 8 is 'b' after the 3-byte U+FEFF (cu 1).
    assert_eq!(line1.char_map.byte_to_line_cu(5), (1, 0));
    assert_eq!(line1.char_map.byte_to_line_cu(8), (1, 1));
    fs::remove_file(&p).ok();
}

#[test]
fn file_shrink_during_read_is_error_not_panic() {
    // I-1: external truncation between indexing and chunk read surfaces as an
    // error rather than slicing past the buffer.
    let p = write_tmp(b"a\nb\nc\nd\n");
    let mut doc = DocFile::open(&p, 1 << 20).unwrap();
    assert_eq!(doc.line_count(), 5);
    fs::write(&p, b"a\n").unwrap(); // shrink after open
    let err = doc.chunk_at(0, 5).unwrap_err();
    assert!(matches!(err, CoreError::Io(_)), "got {err:?}");
    fs::remove_file(&p).ok();
}

#[test]
fn chunk_at_line_start_out_of_range() {
    let p = write_tmp(b"one\ntwo\n");
    let mut doc = DocFile::open(&p, 1 << 20).unwrap();
    let n = doc.line_count();
    assert!(matches!(
        doc.chunk_at(n, 1),
        Err(CoreError::LineOutOfRange)
    ));
    fs::remove_file(&p).ok();
}

#[test]
fn open_rejects_file_over_size_limit() {
    let p = write_tmp(b"0123456789");
    let err = DocFile::open(&p, 5);
    assert!(matches!(err, Err(CoreError::TooLarge)));
    fs::remove_file(&p).ok();
}

#[test]
fn set_encoding_keeps_line_numbers() {
    // Line numbers are byte-based, so switching the display encoding must not
    // change the count (spec §5.2). ASCII is a valid prefix of both codecs.
    let p = write_tmp(b"alpha\nbravo\ncharlie\n");
    let mut doc = DocFile::open(&p, 1 << 20).unwrap();
    let before = doc.line_count();
    assert_eq!(before, 4);
    doc.set_encoding(EncodingLabel::Gbk);
    assert_eq!(doc.line_count(), before, "line count is encoding-independent");
    doc.set_encoding(EncodingLabel::Utf8);
    assert_eq!(doc.line_count(), before);
    fs::remove_file(&p).ok();
}

#[test]
fn utf16le_open_uses_aligned_index() {
    let text = "ab线\ncd\n";
    let p = write_tmp(&encode_all(EncodingLabel::Utf16Le, text).bytes);
    let mut doc = DocFile::open(&p, 1 << 20).unwrap();
    assert_eq!(doc.encoding(), EncodingLabel::Utf16Le);
    // three segments: "ab线\n", "cd\n", trailing empty
    assert_eq!(doc.line_count(), 3);
    let c = doc.chunk_at(0, doc.line_count()).unwrap();
    assert_eq!(c.text, text);
    assert_eq!(c.invalid_count, 0);
    fs::remove_file(&p).ok();
}
