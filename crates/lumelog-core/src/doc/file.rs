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
use crate::error::CoreError;

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

/// Build the line index. UTF-16 uses the aligned 16-bit scan (Review Focus 5)
/// over the whole buffer; byte encodings stream in 8MB windows via memchr.
fn build_index(file: &mut File, size: u64, label: EncodingLabel) -> Result<LineIndex, CoreError> {
    match label {
        EncodingLabel::Utf16Le | EncodingLabel::Utf16Be => {
            let data = read_at(file, 0, size as usize)?;
            let little_endian = label == EncodingLabel::Utf16Le;
            line_index::build_aligned(&data, 0x000A, little_endian)
        }
        _ => {
            let mut b = LineIndexBuilder::new(size);
            let mut buf = vec![0u8; READ_CHUNK as usize];
            let mut off = 0u64;
            while off < size {
                let want = READ_CHUNK.min(size - off) as usize;
                let n = read_into(file, off, &mut buf[..want])?;
                if n == 0 {
                    break;
                }
                b.push_chunk(&buf[..n]);
                off += n as u64;
            }
            b.finish()
        }
    }
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
