//! Generic regex source (spec §7.3.3).
//!
//! Lets a user point LumeLog at an arbitrary one-frame-per-line log dialect by
//! supplying a regex with named capture groups:
//! - `id`   (required) — CAN id, hex, tolerates a `0x`/`0X` prefix
//! - `data` (required) — payload hex, tolerates space / `:` / `,` / `-` separators;
//!   `dlc` is the resulting byte count (hex length / 2)
//! - `ts`   (optional) — timestamp, parsed as `f64`; absent → `None`
//!
//! Construction fails fast if `id`/`data` are missing (`CanError::BadRegex`) or the
//! pattern won't compile (`CanError::RegexCompile`). At run time, a line that does
//! not match, or matches but yields non-hex/odd-length data, is skipped and counted —
//! never a panic, never a stall of the whole stream.

use super::{hex_to_bytes, FrameSource, Line};
use crate::error::CanError;
use crate::frame::{Frame, FrameFlags, SourceStats};
use regex::Regex;

#[derive(Debug)]
pub struct RegexSource<I> {
    lines: I,
    re: Regex,
    decoded: u64,
    skipped: u64,
}

impl<I: Iterator<Item = Line>> RegexSource<I> {
    pub fn new(lines: I, pattern: &str) -> Result<Self, CanError> {
        let re = Regex::new(pattern).map_err(|e| CanError::RegexCompile(e.to_string()))?;
        let mut has_id = false;
        let mut has_data = false;
        for name in re.capture_names().flatten() {
            match name {
                "id" => has_id = true,
                "data" => has_data = true,
                _ => {}
            }
        }
        if !has_id || !has_data {
            return Err(CanError::BadRegex(
                "pattern must define named groups `id` and `data` (optional `ts`)".to_string(),
            ));
        }
        Ok(Self { lines, re, decoded: 0, skipped: 0 })
    }
}

impl<I: Iterator<Item = Line>> FrameSource for RegexSource<I> {
    fn next_frame(&mut self) -> Option<Frame> {
        for (lineno, text) in self.lines.by_ref() {
            if let Some(frame) = parse_line(&self.re, lineno, &text) {
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

fn parse_line(re: &Regex, lineno: u64, text: &str) -> Option<Frame> {
    let caps = re.captures(text)?;

    let id_raw = caps.name("id")?.as_str().trim();
    let id_hex =
        id_raw.strip_prefix("0x").or_else(|| id_raw.strip_prefix("0X")).unwrap_or(id_raw);
    let can_id = u32::from_str_radix(id_hex, 16).ok()?;

    let data_raw = caps.name("data")?.as_str();
    let data_hex: String =
        data_raw.chars().filter(|c| !matches!(c, ' ' | ':' | ',' | '-')).collect();
    let data = hex_to_bytes(&data_hex)?;
    if data.len() > 64 {
        return None;
    }

    let timestamp = caps.name("ts").and_then(|m| m.as_str().parse::<f64>().ok());

    let mut arr = [0u8; 64];
    arr[..data.len()].copy_from_slice(&data);
    Some(Frame {
        line_number: lineno,
        timestamp,
        channel: 0,
        can_id,
        dlc: data.len() as u8,
        data: arr,
        flags: FrameFlags::default(),
    })
}
