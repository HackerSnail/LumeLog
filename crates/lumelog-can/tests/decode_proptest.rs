//! Property tests for the signal decoder (spec §13.1 / Review Focus 1 & 2).
//!
//! These are self-consistency / bound properties that don't need cantools. The
//! Motorola bit-order *correctness* is pinned by `bitorder_golden.rs` (external
//! authority); here we check the Intel extraction against an independent
//! reference formulation, and the signed/physical transforms against their
//! mathematical definitions.

use lumelog_can::dbc::{ByteOrder, Signal};
use lumelog_can::decode::{extract_raw, raw_to_signed, to_physical};
use proptest::prelude::*;

fn intel_signal(start_bit: u32, length: u32) -> Signal {
    Signal {
        name: "s".into(),
        start_bit,
        length,
        byte_order: ByteOrder::LittleEndian,
        is_signed: false,
        factor: 1.0,
        offset: 0.0,
        minimum: 0.0,
        maximum: 0.0,
        unit: String::new(),
        receivers: vec![],
        value_table: vec![],
        mux: lumelog_can::dbc::Mux::NonMuxed,
    }
}

/// Independent Intel reference: read bit `start_bit+k` (LSB-first) into result bit k.
fn intel_reference(data: &[u8], start_bit: u32, length: u32) -> u64 {
    let mut raw = 0u64;
    for k in 0..length {
        let idx = (start_bit + k) as usize;
        if idx / 8 >= data.len() {
            break;
        }
        let bit = (data[idx / 8] >> (idx % 8)) & 1;
        raw |= (bit as u64) << k;
    }
    raw
}

proptest! {
    // Intel extraction equals the direct definition, for any in-range layout.
    #[test]
    fn intel_matches_reference(start_bit in 0u32..44, length in 1u32..=16, bytes in proptest::array::uniform8(any::<u8>())) {
        let data = bytes.to_vec();
        let got = extract_raw(&data, &intel_signal(start_bit, length));
        let want = intel_reference(&data, start_bit, length);
        prop_assert_eq!(got, want);
    }

    // A length-L unsigned field never exceeds 2^L - 1.
    #[test]
    fn intel_within_bit_width(length in 1u32..=32, bytes in proptest::array::uniform8(any::<u8>())) {
        let data = bytes.to_vec();
        let raw = extract_raw(&data, &intel_signal(0, length));
        let mask = if length >= 64 { u64::MAX } else { (1u64 << length) - 1 };
        prop_assert_eq!(raw & !mask, 0, "extraction leaked beyond width");
    }

    // raw_to_signed round-trips: re-masking the signed value returns the raw bits.
    #[test]
    fn signed_roundtrip(raw in any::<u64>(), length in 1u32..=63) {
        let mask = (1u64 << length) - 1;
        let unsigned = raw & mask;
        let signed = raw_to_signed(unsigned, length);
        // The signed value's low `length` bits equal the original bits.
        prop_assert_eq!((signed as u64) & mask, unsigned);
        // Sign is set iff the top bit of the width is set.
        let top = (unsigned >> (length - 1)) & 1 == 1;
        prop_assert_eq!(signed < 0, top);
    }

    // to_physical is exactly raw*factor + offset.
    #[test]
    fn physical_is_linear(raw in -100000i64..100000i64, factor in -1000.0f64..1000.0, offset in -1000.0f64..1000.0) {
        let mut sig = intel_signal(0, 16);
        sig.factor = factor;
        sig.offset = offset;
        let phys = to_physical(raw, &sig);
        let want = (raw as f64) * factor + offset;
        let tol = 1e-6 * want.abs().max(1.0);
        prop_assert!((phys - want).abs() <= tol, "{phys} vs {want}");
    }
}

/// A hand-built over-long signal (which `dbc::parse` now rejects) must still not
/// panic in the public `extract_raw` — the defensive clamp caps the shift at 64.
#[test]
fn overlong_signal_does_not_panic() {
    let data = [0u8; 8];
    let mut sig = intel_signal(0, 65);
    let _ = extract_raw(&data, &sig); // must not panic
    sig.byte_order = ByteOrder::BigEndian;
    let _ = extract_raw(&data, &sig);
    // raw_to_signed on an out-of-range width returns raw unchanged, no underflow.
    let _ = raw_to_signed(0xFFFF_FFFF_FFFF_FFFF, 100);
}
