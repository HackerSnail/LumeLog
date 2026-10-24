//! Deterministic synthetic-candump generator, shared by the 100k acceptance
//! test (`tests/cli_can.rs`) and the throughput bench. Public so integration
//! tests (which only see the lib's public API) can call it. Pure data — no I/O.

/// A generated line plus the frame fields it *must* decode to, so callers can
/// verify faithfulness rather than trusting the parser they just fed.
#[derive(Debug, Clone)]
pub struct GenFrame {
    pub line_number: u64,
    pub text: String,
    pub can_id: u32,
    pub data: Vec<u8>,
}

/// Tiny deterministic LCG (no_std-friendly, reproducible across platforms).
struct Lcg(u64);

impl Lcg {
    fn next_u64(&mut self) -> u64 {
        // Knuth MMIX constants.
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 11
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }
}

/// Produce `n` valid candump ASCII lines from `seed`. Standard 11-bit ids,
/// dlc 1..=8, one of four interfaces. Never emits a blank/garbage line, so a
/// correct parser reports `skipped == 0`.
pub fn gen_candump(n: u64, seed: u64) -> Vec<GenFrame> {
    let mut rng = Lcg(seed);
    let mut out = Vec::with_capacity(n as usize);
    for lineno in 1..=n {
        let can_id = rng.below(0x800) as u32; // 0..=0x7FF — 1 to 3 hex chars
        let dlc = 1 + rng.below(8) as usize; // 1..=8
        let data: Vec<u8> = (0..dlc).map(|_| rng.below(256) as u8).collect();
        let chan = rng.below(4);
        let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
        let text = format!("can{chan} {can_id:X}#{hex}");
        out.push(GenFrame { line_number: lineno, text, can_id, data });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_a_seed() {
        let a = gen_candump(50, 0x1234);
        let b = gen_candump(50, 0x1234);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.text, y.text);
        }
    }

    #[test]
    fn lines_are_nonempty_and_have_hash() {
        for f in gen_candump(100, 99) {
            assert!(f.text.contains('#'));
            assert!(!f.data.is_empty());
            assert!(f.can_id <= 0x7FF);
        }
    }
}
