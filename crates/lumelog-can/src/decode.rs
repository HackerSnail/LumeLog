//! Signal bit-order decoding (spec §7.2).
//!
//! Intel (`@1`, little-endian): `start_bit` is the LSB; bits run `start_bit + k`,
//! byte-major, into result bit `k`.
//!
//! Motorola (`@0`, big-endian / DBC "sawtooth"): `start_bit` is the MSB in DBC
//! numbering (`byte = start_bit/8`, within-byte bit `= start_bit%8`, where bit 7
//! is that byte's MSB). Bits walk *down* to bit 0 of the current byte, then jump
//! to bit 7 of the next *higher* byte. This is the highest-risk part of M2 and is
//! pinned bit-for-bit against the cantools-generated `golden_bitorder.json`
//! (`tests/bitorder_golden.rs`); the golden is the authority, this code conforms
//! to it.

use crate::dbc::{ByteOrder, Message, Signal};
use crate::frame::SignalValue;

/// Extract the `sig.length` raw bits for `sig` from `data` as an unsigned value.
pub fn extract_raw(data: &[u8], sig: &Signal) -> u64 {
    match sig.byte_order {
        ByteOrder::LittleEndian => extract_intel(data, sig.start_bit, sig.length),
        ByteOrder::BigEndian => extract_motorola(data, sig.start_bit, sig.length),
    }
}

fn extract_intel(data: &[u8], start_bit: u32, length: u32) -> u64 {
    let mut raw = 0u64;
    for k in 0..length {
        let idx = (start_bit + k) as usize;
        let byte = idx / 8;
        if byte >= data.len() {
            break;
        }
        let bit = (idx % 8) as u32;
        raw |= (((data[byte] >> bit) & 1) as u64) << k;
    }
    raw
}

fn extract_motorola(data: &[u8], start_bit: u32, length: u32) -> u64 {
    let mut raw = 0u64;
    let mut byte = (start_bit / 8) as usize;
    let mut bit = (start_bit % 8) as i32;
    for k in 0..length {
        if byte >= data.len() {
            break;
        }
        let v = ((data[byte] >> (bit as u32)) & 1) as u64;
        // First bit walked is the signal's MSB.
        raw |= v << (length - 1 - k);
        if bit == 0 {
            bit = 7;
            byte += 1;
        } else {
            bit -= 1;
        }
    }
    raw
}

/// Interpret the low `length` bits of `raw` as a two's-complement signed value.
pub fn raw_to_signed(raw: u64, length: u32) -> i64 {
    if length == 0 || length >= 64 {
        return raw as i64;
    }
    let shift = 64 - length;
    // Move the sign bit to position 63, arithmetic-shift back to sign-extend.
    ((raw as i64) << shift) >> shift
}

/// `physical = raw * factor + offset` (spec §7.2; f64 throughout).
pub fn to_physical(raw_signed: i64, sig: &Signal) -> f64 {
    (raw_signed as f64) * sig.factor + sig.offset
}

/// Decode every signal of `msg` from `data`. Mux filtering (only the active
/// multiplexed signals) happens downstream in the CLI / IPC layer; this returns
/// all signals with `value_table` matches filled into `enum_text`.
pub fn decode_signals(msg: &Message, data: &[u8]) -> Vec<SignalValue> {
    msg.signals
        .iter()
        .map(|sig| {
            let raw_u = extract_raw(data, sig);
            let raw_i = if sig.is_signed { raw_to_signed(raw_u, sig.length) } else { raw_u as i64 };
            let physical = to_physical(raw_i, sig);
            let enum_text =
                sig.value_table.iter().find(|(v, _)| *v == raw_i).map(|(_, t)| t.clone());
            SignalValue {
                name: sig.name.clone(),
                raw: raw_i,
                physical,
                enum_text,
                unit: sig.unit.clone(),
            }
        })
        .collect()
}
