//! Streaming CAN-log sources (spec §7.3). Each source turns a line iterator
//! `((line_number, text))` into `Frame`s, skipping and counting malformed lines
//! without ever panicking or stalling the stream.

pub mod candump;

use crate::frame::{Frame, SourceStats};

/// One source line paired with its 1-based number in the underlying file.
pub type Line = (u64, String);

/// A frame-emitting iterator over some CAN log dialect.
pub trait FrameSource {
    /// Next successfully decoded frame; internal skips advance past bad lines.
    fn next_frame(&mut self) -> Option<Frame>;
    /// Tallies of frames emitted vs. non-blank lines skipped so far.
    fn stats(&self) -> SourceStats;
}

/// Parse an even-length hex string into bytes. `None` on odd length or a non-hex
/// character; empty input is an empty payload. Shared by every source.
pub(crate) fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

pub use candump::CandumpSource;
