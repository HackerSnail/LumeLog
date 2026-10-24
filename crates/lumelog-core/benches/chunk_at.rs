//! Chunk-fetch latency benchmark (spec P4: 取块+解码 ≤16ms P95). Manual run,
//! not part of CI: `cargo bench -p lumelog-core -- chunk_at_gb`
//!
//! Builds a ~1 GiB deterministic log, opens it once (index build is setup, not
//! measured), then fires 1000 random `chunk_at` calls (a scroll burst at
//! ~600-line windows) and reports the wall-clock P95. Criterion gets the same
//! call as a normal bench for trend tracking; the printed P95 is the number
//! that goes into the CHANGELOG.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use lumelog_core::doc::DocFile;
use lumelog_core::tests_common::gen_log;
use std::io::Write;
use std::time::Instant;

const WINDOW_LINES: usize = 600;
const SAMPLES: usize = 1000;

/// Deterministic LCG — no rand dependency; seeds differ enough to defeat the
/// 64 MB chunk LRU and exercise the read+decode path each call.
fn next_rng(x: &mut u64) -> u64 {
    *x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *x >> 11
}

fn make_doc() -> (std::path::PathBuf, DocFile) {
    let bytes = gen_log(1024); // ~1 GiB of ASCII HIL log lines
    let mut path = std::env::temp_dir();
    path.push(format!("lumelog_chunk_gb_{}.log", std::process::id()));
    {
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&bytes).unwrap();
    }
    let doc = DocFile::open(&path, 2u64 << 30).unwrap();
    (path, doc)
}

fn chunk_at_gb(c: &mut Criterion) {
    let (path, mut doc) = make_doc();
    let lines = doc.line_count();

    // The P95 gate: 1000 random single-window fetches, each timed on its own.
    let mut rng = 0x9E3779B97F4A7C15u64;
    let mut times: Vec<f64> = Vec::with_capacity(SAMPLES);
    let mut decoded_bytes: u64 = 0;
    for _ in 0..SAMPLES {
        let start = (next_rng(&mut rng) as usize) % lines.saturating_sub(WINDOW_LINES);
        let t = Instant::now();
        let chunk = doc.chunk_at(start, WINDOW_LINES).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        decoded_bytes += chunk.text.len() as u64;
        black_box(chunk.start_line);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = times[SAMPLES / 2];
    let p95 = times[(SAMPLES as f64 * 0.95).floor() as usize - 1];
    let p99 = times[(SAMPLES as f64 * 0.99).floor() as usize - 1];
    println!(
        "chunk_at_gb: {} random {}-line windows over ~1 GiB, {} MB decoded\n  \
         p50 = {p50:.3} ms  p95 = {p95:.3} ms  p99 = {p99:.3} ms  (gate: p95 <= 16 ms)",
        SAMPLES,
        WINDOW_LINES,
        decoded_bytes / 1_000_000,
    );

    c.bench_function("chunk_at_gb", |b| {
        let mut rng = 0xDEADBEEF12345678u64;
        b.iter(|| {
            let start = (next_rng(&mut rng) as usize) % lines.saturating_sub(WINDOW_LINES);
            let chunk = doc.chunk_at(black_box(start), WINDOW_LINES).unwrap();
            black_box(chunk.text.len())
        })
    });

    std::fs::remove_file(&path).ok();
}

criterion_group!(benches, chunk_at_gb);
criterion_main!(benches);
