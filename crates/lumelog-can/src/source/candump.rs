//! candump ASCII source (spec §7.3.1).
//!
//! Accepted line shapes (whitespace-separated):
//! - `(1.234567) can0 123#DEADBEEF` — optional `()` timestamp, iface, `id#data`
//! - `can0 735#DEADBEEF`            — no timestamp → `None`
//! - `can0 123#R`                   — RTR (`is_remote`, empty data)
//! - `can0 123##03112233...`        — FD: `##` + 2-hex flags byte + data hex
//!
//! Anything else (error frames, `garbage`, non-hex payload) is skipped and
//! counted. Channel is the trailing digit run of the iface (`can2` → 2).

use super::{hex_to_bytes, FrameSource, Line};
use crate::frame::{Frame, FrameFlags, SourceStats};

pub struct CandumpSource<I> {
    lines: I,
    decoded: u64,
    skipped: u64,
}

impl<I: Iterator<Item = Line>> CandumpSource<I> {
    pub fn new(lines: I) -> Self {
        Self { lines, decoded: 0, skipped: 0 }
    }
}

impl<I: Iterator<Item = Line>> FrameSource for CandumpSource<I> {
    fn next_frame(&mut self) -> Option<Frame> {
        for (lineno, text) in self.lines.by_ref() {
            if let Some(frame) = parse_line(lineno, &text) {
                self.decoded += 1;
                return Some(frame);
            }
            // Blank lines are structural, not skipped frames; only count a real
            // line we failed to decode.
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

fn iface_channel(iface: &str) -> u8 {
    let digits: String = iface
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    digits.parse::<u32>().map(|v| v as u8).unwrap_or(0)
}

fn parse_line(lineno: u64, text: &str) -> Option<Frame> {
    let toks: Vec<&str> = text.split_whitespace().collect();
    let pidx = toks.iter().position(|t| t.contains('#'))?;
    let payload = toks[pidx];

    let mut timestamp = None;
    for t in &toks[..pidx] {
        if let Some(inner) = t.strip_prefix('(') {
            if let Ok(v) = inner.trim_end_matches(')').parse::<f64>() {
                timestamp = Some(v);
                break;
            }
        }
    }

    let mut channel = 0u8;
    for t in toks[..pidx].iter().rev() {
        if !t.starts_with('(') {
            channel = iface_channel(t);
            break;
        }
    }

    let (id_hex, rest) = payload.split_once('#')?;
    let mut flags = FrameFlags::default();
    let data: Vec<u8>;
    if let Some(fdrest) = rest.strip_prefix('#') {
        // CAN-FD: `##` then a 2-hex flags byte then the data hex.
        flags.is_fd = true;
        if fdrest.len() < 2 {
            return None;
        }
        let fb = u8::from_str_radix(&fdrest[..2], 16).ok()?;
        flags.brs = fb & 0x01 != 0;
        flags.esi = fb & 0x02 != 0;
        data = hex_to_bytes(&fdrest[2..])?;
    } else if rest.eq_ignore_ascii_case("R") {
        flags.is_remote = true;
        data = Vec::new();
    } else {
        data = hex_to_bytes(rest)?;
    }

    if data.len() > 64 {
        return None;
    }
    let can_id = u32::from_str_radix(id_hex, 16).ok()?;
    let mut arr = [0u8; 64];
    arr[..data.len()].copy_from_slice(&data);
    Some(Frame {
        line_number: lineno,
        timestamp,
        channel,
        can_id,
        dlc: data.len() as u8,
        data: arr,
        flags,
    })
}
