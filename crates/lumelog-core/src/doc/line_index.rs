//! Line index: maps line numbers to byte offsets in the raw file.
//! Line boundaries are byte-based (independent of encoding), which is why
//! switching encoding keeps line numbers stable (spec §5.2).

use crate::error::CoreError;

/// Byte offset index of line starts. `offsets[i]` = first byte of line `i`.
/// Line `i` occupies `[offsets[i], offsets[i+1])` (or `file_size` for the last).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    offsets: Vec<u32>,
    file_size: u64,
}

impl LineIndex {
    pub fn line_count(&self) -> usize {
        self.offsets.len()
    }

    /// Byte range `[start, end)` of line `n`, inclusive of its trailing `\n`.
    pub fn line_range(&self, n: usize) -> Option<(u64, u64)> {
        let start = *self.offsets.get(n)? as u64;
        let end = if n + 1 < self.offsets.len() {
            self.offsets[n + 1] as u64
        } else {
            self.file_size
        };
        Some((start, end))
    }

    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// 0-based line number containing the absolute byte offset `byte`.
    /// Uses binary search over line start offsets.
    pub fn line_at_byte(&self, byte: u64) -> usize {
        // partition_point: number of offsets <= byte, minus 1, clamped.
        let mut idx = self.offsets.partition_point(|&o| (o as u64) <= byte);
        if idx == 0 {
            0
        } else {
            idx -= 1;
            idx
        }
    }
}

/// Build a line index from a full in-memory buffer (tests + CLI convenience).
pub fn build(data: &[u8]) -> Result<LineIndex, CoreError> {
    let mut b = LineIndexBuilder::new(data.len() as u64);
    b.push_chunk(data);
    b.finish()
}

/// Build a UTF-16 line index: only 16-bit code units equal to `pattern`
/// (0x000A = LF) that sit at an EVEN byte offset count as line breaks.
/// Guards against `0x0A` appearing as the high byte of an unrelated code unit.
pub fn build_aligned(
    data: &[u8],
    pattern: u16,
    little_endian: bool,
) -> Result<LineIndex, CoreError> {
    let mut offsets: Vec<u32> = vec![0];
    let mut i = 0usize;
    while i + 1 < data.len() {
        // i is always even here; only aligned code units are considered.
        let unit = if little_endian {
            u16::from_le_bytes([data[i], data[i + 1]])
        } else {
            u16::from_be_bytes([data[i], data[i + 1]])
        };
        if unit == pattern {
            let next = (i + 2) as u64;
            if next > u32::MAX as u64 {
                return Err(CoreError::TooLarge);
            }
            offsets.push(next as u32);
        }
        i += 2;
    }
    Ok(LineIndex {
        offsets,
        file_size: data.len() as u64,
    })
}

/// Streaming builder fed fixed-size chunks (8MB in the app). Produces the same
/// offsets as `build` for the concatenated bytes (pinned by test).
pub struct LineIndexBuilder {
    offsets: Vec<u32>,
    abs_pos: u64, // absolute offset of the next unread byte (cumulative)
    file_size: u64,
    overflow: bool, // a line start exceeded u32 range (=> file too large to index)
}

impl LineIndexBuilder {
    pub fn new(file_size: u64) -> Self {
        Self {
            offsets: vec![0],
            abs_pos: 0,
            file_size,
            overflow: false,
        }
    }

    pub fn push_chunk(&mut self, bytes: &[u8]) {
        for local in memchr::memchr_iter(b'\n', bytes) {
            let nl_abs = self.abs_pos + local as u64; // position of the 0x0A byte
            let start = nl_abs + 1; // first byte of the next line
            if start > u32::MAX as u64 {
                self.overflow = true;
            } else {
                self.offsets.push(start as u32);
            }
        }
        self.abs_pos += bytes.len() as u64;
    }

    pub fn finish(self) -> Result<LineIndex, CoreError> {
        if self.overflow || self.file_size > u32::MAX as u64 {
            return Err(CoreError::TooLarge);
        }
        Ok(LineIndex {
            offsets: self.offsets,
            file_size: self.file_size,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offs(idx: &LineIndex) -> Vec<u32> {
        idx.offsets().to_vec()
    }

    #[test]
    fn empty_file_is_one_line() {
        let idx = build(b"").unwrap();
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.line_range(0), Some((0, 0)));
    }

    #[test]
    fn no_trailing_newline_counts_last_line() {
        let idx = build(b"ab\ncd").unwrap();
        assert_eq!(idx.line_count(), 2);
        assert_eq!(idx.line_range(0), Some((0, 3))); // "ab\n"
        assert_eq!(idx.line_range(1), Some((3, 5))); // "cd"
    }

    #[test]
    fn crlf_belongs_to_previous_line() {
        let idx = build(b"ab\r\ncd").unwrap();
        assert_eq!(idx.line_count(), 2);
        // line 0 spans through the \r\n (split only on 0x0A)
        assert_eq!(idx.line_range(0), Some((0, 4))); // "ab\r\n"
        assert_eq!(idx.line_range(1), Some((4, 6))); // "cd"
    }

    #[test]
    fn utf16le_scan_even_aligned_only() {
        // [0x41,0x0A] = code unit 0x0A41 (0x0A at odd byte -> NOT a break),
        // [0x0A,0x00] = code unit 0x000A (real LF at even byte -> break).
        let data = [0x41u8, 0x0A, 0x0A, 0x00];
        let idx = build_aligned(&data, 0x000A, true).unwrap();
        assert_eq!(idx.line_count(), 2, "only the aligned 0x000A is a break");
        assert_eq!(offs(&idx), vec![0, 4]);
    }

    #[test]
    fn utf16be_scan_even_aligned_only() {
        // Big-endian mirror of the LE test: [0x0A,0x41] = unit 0x0A41 (LF in
        // high byte, NOT a break), [0x00,0x0A] = unit 0x000A (real LF -> break).
        let data = [0x0Au8, 0x41, 0x00, 0x0A];
        let idx = build_aligned(&data, 0x000A, false).unwrap();
        assert_eq!(idx.line_count(), 2, "only the aligned 0x000A is a break");
        assert_eq!(offs(&idx), vec![0, 4]);
    }

    #[test]
    fn builder_rejects_file_beyond_u32_range() {
        // A >4GB file cannot be indexed with u32 offsets; finish must refuse
        // rather than silently truncate. Constructed without allocating 4GB.
        let mut b = LineIndexBuilder::new(u32::MAX as u64 + 1);
        b.push_chunk(b"just some bytes\n");
        assert!(matches!(b.finish(), Err(CoreError::TooLarge)));
    }

    #[test]
    fn builder_chunks_equal_whole() {
        let data = b"line1\nline2\nline3\n\ntrailing".to_vec();
        let whole = build(&data).unwrap();
        for &size in &[1usize, 3, 8 * 1024 * 1024] {
            let mut b = LineIndexBuilder::new(data.len() as u64);
            for chunk in data.chunks(size.max(1)) {
                b.push_chunk(chunk);
            }
            assert_eq!(b.finish().unwrap(), whole, "chunk size {size}");
        }
    }

    #[test]
    fn scanner_splits_on_every_lf_regardless_of_bytes() {
        // The raw scanner is content-agnostic: it splits on every 0x0A. Note
        // this is NOT a valid GB18030 4-byte sequence — GB18030 trail bytes are
        // 0x30-0x39, so 0x0A can never be one; for every supported codec 0x0A
        // is only ever a real newline. The bytes are arbitrary and just pin the
        // split / no-panic behavior; display merge-retry lives in DocFile.
        let data = [0x81u8, 0x30, 0x88, 0x0A, b'x', b'\n'];
        let idx = build(&data).unwrap();
        // 0x0A at index 3 splits (line1 starts at 4), final 0x0A at index 5
        // splits again (line2 starts at 6, empty) => 3 lines
        assert_eq!(idx.line_count(), 3);
        assert_eq!(idx.line_range(2), Some((6, 6)));
    }

    #[test]
    fn line_at_byte_maps_to_containing_line() {
        let idx = build(b"aaa\nbb\ncccc").unwrap();
        assert_eq!(idx.line_at_byte(0), 0);
        assert_eq!(idx.line_at_byte(3), 0); // the \n at index 3 belongs to line 0
        assert_eq!(idx.line_at_byte(4), 1);
        assert_eq!(idx.line_at_byte(7), 2);
    }
}
