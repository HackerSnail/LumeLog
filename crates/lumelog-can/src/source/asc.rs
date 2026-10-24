//! Vector ASC source (spec §7.3.2).
//!
//! Accepted frame-line shape (whitespace-separated):
//! - `1.000000 1 123 Rx d 8 DE AD BE EF ...` — `<base> <chan> <ID> <Rx|Tx> <type> ...`
//! - `2.500000 2 1A7x Rx d 4 11 22 33 44`     — id `x` suffix marks an extended frame
//! - `3.000000 1 321 Tx r 3`                  — `r` type → remote (declared dlc, no bytes)
//!
//! A line is a frame candidate only when its first token parses as an `f64` base
//! timestamp and its direction token is `Rx`/`Tx`. `boe`/`bcs`/`error` and any
//! other type token, plus headers (`date`/`base`) and non-frame lines, are skipped
//! and counted. ASC has many dialects; only the shapes above are guaranteed —
//! see the CHANGELOG known-limitations note.

use super::{FrameSource, Line};
use crate::frame::{Frame, FrameFlags, SourceStats};

pub struct AscSource<I> {
    lines: I,
    decoded: u64,
    skipped: u64,
}

impl<I: Iterator<Item = Line>> AscSource<I> {
    pub fn new(lines: I) -> Self {
        Self { lines, decoded: 0, skipped: 0 }
    }
}

impl<I: Iterator<Item = Line>> FrameSource for AscSource<I> {
    fn next_frame(&mut self) -> Option<Frame> {
        for (lineno, text) in self.lines.by_ref() {
            if let Some(frame) = parse_line(lineno, &text) {
                self.decoded += 1;
                return Some(frame);
            }
            if !text.trim().is_empty() {
                self.skipped += 1;
            }
        }
        None
    }

    fn stats(&self) -> SourceStats {
        SourceStats { decoded: self.decoded, skipped: self.skipped }
    }
}

/// Strip a trailing `x`/`X` extended-frame marker and parse the id as hex.
fn parse_id(tok: &str) -> Option<u32> {
    let hex = tok.strip_suffix(['x', 'X']).unwrap_or(tok);
    u32::from_str_radix(hex, 16).ok()
}

fn parse_line(lineno: u64, text: &str) -> Option<Frame> {
    let toks: Vec<&str> = text.split_whitespace().collect();
    if toks.len() < 5 {
        return None;
    }
    // Frame lines begin with a numeric base timestamp; headers like `date`/`base`
    // and `CAN`-prefixed lines fail here and are skipped.
    let timestamp = toks[0].parse::<f64>().ok()?;
    if !matches!(toks[3], "Rx" | "Tx" | "rx" | "tx") {
        return None;
    }
    let channel = toks[1].parse::<u32>().ok()? as u8;
    let can_id = parse_id(toks[2])?;

    let mut flags = FrameFlags::default();
    let data: Vec<u8>;
    let dlc: u8;
    match toks[4] {
        "d" | "D" | "data" | "DATA" => {
            // `<dlc> <b0> <b1> ...` — one hex byte per trailing token; declared dlc
            // (toks[5]) is informational, dlc is the actual byte count.
            let bytes = toks.get(6..).unwrap_or(&[]);
            data = bytes.iter().map(|t| u8::from_str_radix(t, 16)).collect::<Result<_, _>>().ok()?;
            if data.len() > 64 {
                return None;
            }
            dlc = data.len() as u8;
        }
        "r" | "R" | "remote" | "REMOTE" => {
            flags.is_remote = true;
            data = Vec::new();
            dlc = toks.get(5).and_then(|t| t.parse::<u8>().ok())?;
        }
        // `boe`, `bcs`, `error`, … — recognized but not decoded (skipped).
        _ => return None,
    }

    let mut arr = [0u8; 64];
    arr[..data.len()].copy_from_slice(&data);
    Some(Frame { line_number: lineno, timestamp: Some(timestamp), channel, can_id, dlc, data: arr, flags })
}
