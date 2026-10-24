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
