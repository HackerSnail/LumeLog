//! Open-file handle: raw file + line index + encoding + a bounded LRU of
//! decoded chunks (spec §4.2). Pure Rust, no async — the Tauri command layer
//! calls these from a blocking task.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::doc::chunk::{build_chunk, chunk_size, CharMap, Chunk};
use crate::doc::line_index::{self, LineIndex, LineIndexBuilder};
use crate::doc::save::{detect_eol, EolStyle};
use crate::encoding::{self, EncodingLabel};
use crate::error::{CoreError, OpenReject};

/// Upper bound on the decoded-chunk cache, per spec §4.2.
const CACHE_LIMIT_BYTES: usize = 64 * 1024 * 1024;
/// Streaming read granularity for index building and chunk reads.
const READ_CHUNK: u64 = 8 * 1024 * 1024;

/// A document opened from disk.
pub struct DocFile {
    file: File,
    path: PathBuf,
    index: LineIndex,
    label: EncodingLabel,
    eol_style: EolStyle,
    mtime: SystemTime,
    size: u64,
    /// Decoded-chunk LRU: front = least-recently-used (evicted), back = most.
    cache: VecDeque<(u64, Chunk)>,
    cache_bytes: usize,
}

impl DocFile {
    /// Open `path`, detect its encoding, build the line index, refusing files
    /// larger than `size_limit` (spec §4.1: oversized files are rejected).
    pub fn open(path: &Path, size_limit: u64) -> Result<DocFile, CoreError> {
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        let size = meta.len();
        if size > size_limit {
            return Err(CoreError::TooLarge);
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        // Detect from the leading window (spec §5.2: first up-to-64KB).
        let head = read_at(&mut file, 0, (size as usize).min(64 * 1024))?;
        let label = encoding::detect(&head);
        // Record the dominant line-ending style from the decoded leading window
        // (spec §4.4: on open we detect CRLF/LF and store it as document metadata).
        let eol_style = detect_eol(&encoding::decode_all(label, &head).text);

        let index = build_index(&mut file, size, label)?;

        Ok(DocFile {
            file,
            path: path.to_path_buf(),
            index,
            label,
            eol_style,
            mtime,
            size,
            cache: VecDeque::new(),
            cache_bytes: 0,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn encoding(&self) -> EncodingLabel {
        self.label
    }
    /// Dominant line-ending style detected on open (spec §4.4).
    pub fn eol_style(&self) -> EolStyle {
        self.eol_style
    }
    pub fn line_count(&self) -> usize {
        self.index.line_count()
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    pub fn mtime(&self) -> SystemTime {
        self.mtime
    }
    /// `(size, mtime)` snapshot for external-modification / conflict checks.
    pub fn disk_meta(&self) -> (u64, SystemTime) {
        (self.size, self.mtime)
    }

    // --- crate-internal surface for the search scanner (crate::search) ---

    /// Read up to `len` raw bytes at absolute offset `pos` (short read at EOF).
    /// Search scans the file in fixed windows through this, without decoding.
    pub(crate) fn read_window(&mut self, pos: u64, len: usize) -> Result<Vec<u8>, CoreError> {
        read_at(&mut self.file, pos, len)
    }
    /// 0-based line number containing the absolute byte offset `byte`.
    pub(crate) fn line_at_byte(&self, byte: u64) -> usize {
        self.index.line_at_byte(byte)
    }

    /// Raw byte range `[start, end)` of line `n` (search maps a hit's absolute
    /// byte to a within-line offset through this).
    pub(crate) fn line_byte_range(&self, n: usize) -> Option<(u64, u64)> {
        self.index.line_range(n)
    }

    /// Switch the display encoding. Line numbers are byte-based and stay put
    /// (spec §5.2); only the decoded text of chunks changes, so drop the cache.
    pub fn set_encoding(&mut self, label: EncodingLabel) {
        self.label = label;
        self.cache.clear();
        self.cache_bytes = 0;
    }

    /// Rebind the handle after we replaced the file on disk (an in-place save
    /// `rename`s over `path`, so the old `File` still points at the discarded
    /// inode and the old index has stale byte offsets). Reopen, re-detect the
    /// EOL style from the freshly written head, and rebuild the index against
    /// the bytes we just wrote — otherwise `chunk_at` would serve pre-save data.
    pub(crate) fn reload_after_save(&mut self, encoding: EncodingLabel) -> Result<(), CoreError> {
        let mut file = File::open(&self.path)?;
        let meta = file.metadata()?;
        let size = meta.len();
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let head = read_at(&mut file, 0, (size as usize).min(64 * 1024))?;
        let eol_style = detect_eol(&encoding::decode_all(encoding, &head).text);
        let index = build_index(&mut file, size, encoding)?;

        self.file = file;
        self.size = size;
        self.mtime = mtime;
        self.label = encoding;
        self.eol_style = eol_style;
        self.index = index;
        self.cache.clear();
        self.cache_bytes = 0;
        Ok(())
    }

    /// Decode lines `[line_start, line_start + line_count)` into a [`Chunk`],
    /// serving from the LRU when possible.
    pub fn chunk_at(&mut self, line_start: usize, line_count: usize) -> Result<Chunk, CoreError> {
        if line_start >= self.index.line_count() {
            return Err(CoreError::LineOutOfRange);
        }
        let count = line_count.min(self.index.line_count() - line_start);
        // A zero-line window is legal (empty viewport): hand back an empty
        // chunk instead of underflowing `line_start + count - 1` below.
        if count == 0 {
            return Ok(Chunk {
                start_line: line_start,
                text: String::new(),
                invalid_count: 0,
                char_map: CharMap::from_lines(line_start as u32, self.label, Vec::new()),
            });
        }
        let key = ((line_start as u64) << 32) | (count as u64);

        if let Some(pos) = self.cache.iter().position(|(k, _)| *k == key) {
            // MRU move-to-back, then hand back a copy.
            let entry = self.cache.remove(pos).expect("position from iter");
            let chunk = entry.1.clone();
            self.cache.push_back((key, chunk.clone()));
            return Ok(chunk);
        }

        let (ls, _) = self
            .index
            .line_range(line_start)
            .ok_or(CoreError::LineOutOfRange)?;
        let (_, le) = self
            .index
            .line_range(line_start + count - 1)
            .ok_or(CoreError::LineOutOfRange)?;
        let want = (le - ls) as usize;
        let bytes = read_at(&mut self.file, ls, want)?;
        if bytes.len() != want {
            // The file shrank (rotation / external truncation) since indexing:
            // surface it as an error rather than slicing out of range.
            return Err(CoreError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "file shrank during read",
            )));
        }
        let chunk = build_chunk(&bytes, ls, &self.index, self.label, line_start, count);

        self.cache_bytes += chunk_size(&chunk);
        self.cache.push_back((key, chunk.clone()));
        while self.cache_bytes > CACHE_LIMIT_BYTES {
            if let Some((_, evicted)) = self.cache.pop_front() {
                self.cache_bytes = self.cache_bytes.saturating_sub(chunk_size(&evicted));
            } else {
                break;
            }
        }
        Ok(chunk)
    }
}

/// Build the line index, streaming byte encodings in `chunk_size` windows and
/// reporting cumulative indexed bytes to `on_progress` after each window. The
/// app drives this from a background index thread and forwards progress to the
/// UI (spec §9 `on_index_progress`). UTF-16 uses the aligned 16-bit scan over
/// the whole buffer (Review Focus 5), reporting once at completion. Passing
/// `READ_CHUNK` reproduces the historical synchronous path byte-for-byte.
pub fn build_index_with_progress(
    file: &mut File,
    size: u64,
    label: EncodingLabel,
    chunk_size: u64,
    on_progress: &mut dyn FnMut(u64),
) -> Result<LineIndex, CoreError> {
    match label {
        EncodingLabel::Utf16Le | EncodingLabel::Utf16Be => {
            let data = read_at(file, 0, size as usize)?;
            let little_endian = label == EncodingLabel::Utf16Le;
            let idx = line_index::build_aligned(&data, 0x000A, little_endian)?;
            on_progress(size);
            Ok(idx)
        }
        _ => {
            let chunk = chunk_size.max(1) as usize;
            let mut b = LineIndexBuilder::new(size);
            let mut buf = vec![0u8; chunk];
            let mut off = 0u64;
            while off < size {
                let want = chunk.min((size - off) as usize);
                let n = read_into(file, off, &mut buf[..want])?;
                if n == 0 {
                    break;
                }
                b.push_chunk(&buf[..n]);
                off += n as u64;
                on_progress(off);
            }
            b.finish()
        }
    }
}

/// Synchronous index build used by `DocFile::open` — the no-progress delegate
/// over the shared streaming core, so open and background indexing are one
/// code path (DRY, pinned by `build_index_with_progress` parity tests).
fn build_index(file: &mut File, size: u64, label: EncodingLabel) -> Result<LineIndex, CoreError> {
    build_index_with_progress(file, size, label, READ_CHUNK, &mut |_| {})
}

/// Pre-open validation for the IPC `doc_open` (spec §9): existence, directory,
/// permission and size limit, each returning its own `OpenReject` reason so the
/// app can surface a precise error code rather than a generic io failure. Pure
/// — it only stats the path and opens no handle. `size_limit` is the caller's
/// cap; genuine read failures still surface later through `DocFile::open`.
pub fn validate_open_path(path: &Path, size_limit: u64) -> Result<(), OpenReject> {
    let meta = path.metadata().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => OpenReject::NotFound,
        std::io::ErrorKind::PermissionDenied => OpenReject::PermissionDenied,
        _ => OpenReject::NotFound,
    })?;
    if meta.is_dir() {
        return Err(OpenReject::IsDirectory);
    }
    if meta.len() > size_limit {
        return Err(OpenReject::TooLarge(meta.len(), size_limit));
    }
    Ok(())
}

/// Read exactly `len` bytes (or to EOF) starting at `pos` without disturbing
/// the caller's cursor more than needed. `seek` + `read` loop; serialization is
/// the caller's responsibility (`DocFile` holds a single handle).
fn read_at(file: &mut File, pos: u64, len: usize) -> Result<Vec<u8>, CoreError> {
    let mut out = vec![0u8; len];
    let n = read_into(file, pos, &mut out)?;
    out.truncate(n);
    Ok(out)
}

fn read_into(file: &mut File, pos: u64, buf: &mut [u8]) -> Result<usize, CoreError> {
    file.seek(SeekFrom::Start(pos))?;
    let mut filled = 0usize;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod ipc_logic_tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static SEQ: AtomicUsize = AtomicUsize::new(0);

    fn tmpdir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "lumelog-open-{}-{}-{}",
            tag,
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn validate_open_path_ok_under_limit() {
        let dir = tmpdir("ok");
        let f = dir.join("a.log");
        fs::write(&f, b"line1\nline2\n").unwrap();
        assert_eq!(validate_open_path(&f, 1024), Ok(()));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_open_path_not_found() {
        let dir = tmpdir("nf");
        let missing = dir.join("nope.log");
        assert_eq!(validate_open_path(&missing, 1024), Err(OpenReject::NotFound));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_open_path_is_directory() {
        let dir = tmpdir("dir");
        assert_eq!(validate_open_path(&dir, 1024), Err(OpenReject::IsDirectory));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_open_path_too_large_reports_sizes() {
        let dir = tmpdir("big");
        let f = dir.join("a.log");
        fs::write(&f, b"0123456789").unwrap(); // 10 bytes
        assert_eq!(validate_open_path(&f, 4), Err(OpenReject::TooLarge(10, 4)));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_index_with_progress_matches_whole_and_reports_monotonically() {
        let dir = tmpdir("prog");
        let f = dir.join("log");
        let mut text = String::new();
        for i in 0..1000 {
            text.push_str(&format!("line {i} payload-{:0>4}\n", i % 7));
        }
        fs::write(&f, text.as_bytes()).unwrap();
        let size = text.len() as u64;
        let mut file = fs::File::open(&f).unwrap();

        let mut progress: Vec<u64> = Vec::new();
        let idx = build_index_with_progress(
            &mut file,
            size,
            EncodingLabel::Utf8,
            128, // tiny chunk => many progress steps
            &mut |b| progress.push(b),
        )
        .unwrap();

        // Parity with the whole-buffer builder (same offsets / line count).
        let whole = line_index::build(text.as_bytes()).unwrap();
        assert_eq!(idx.offsets(), whole.offsets());
        assert_eq!(idx.line_count(), whole.line_count());

        // Strictly increasing, terminates exactly at EOF size, more than one step.
        assert!(
            progress.windows(2).all(|w| w[0] < w[1]),
            "progress not monotonic: {progress:?}"
        );
        assert_eq!(*progress.last().unwrap(), size);
        assert!(progress.len() > 1, "expected multiple steps: {progress:?}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn build_index_with_progress_single_report_for_tiny_file() {
        let dir = tmpdir("one");
        let f = dir.join("log");
        fs::write(&f, b"only one line\n").unwrap();
        let size = fs::metadata(&f).unwrap().len();
        let mut file = fs::File::open(&f).unwrap();
        let mut progress: Vec<u64> = Vec::new();
        build_index_with_progress(
            &mut file,
            size,
            EncodingLabel::Utf8,
            READ_CHUNK,
            &mut |b| progress.push(b),
        )
        .unwrap();
        assert_eq!(progress, vec![size]); // one chunk => exactly one report at EOF
        fs::remove_dir_all(&dir).ok();
    }
}
