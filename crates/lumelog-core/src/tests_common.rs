//! Shared test/bench helpers: deterministic log generator.
//! Available to unit tests, integration tests and benches.

/// Generate a synthetic log of `lines` lines, deterministic via a simple
/// linear-congruential PRNG (fixed seed). Each line resembles a HIL bench log.
pub fn gen_log_lines(lines: usize) -> String {
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut out = String::with_capacity(lines * 48);
    for i in 0..lines {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let v = (state >> 33) % 1000;
        out.push_str(&format!(
            "2026-10-24 12:00:{:02}.{:03} INFO  [task-{}] ECUsync ok val={}\n",
            i % 60,
            v,
            i % 8,
            v
        ));
    }
    out
}

/// Bytes of a synthetic log sized to ~`size_mb` (approximate, ASCII lines).
pub fn gen_log(size_mb: usize) -> Vec<u8> {
    let target = size_mb * 1024 * 1024;
    let per_line = 64usize;
    gen_log_lines(target / per_line).into_bytes()
}
