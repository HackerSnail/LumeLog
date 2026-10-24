//! End-to-end CLI tests + the M2 engine 100k-frame acceptance (spec §7.5 / §12).
//!
//! Uses `CARGO_BIN_EXE_lumelog-can` so the *real* built binary is exercised, and
//! `lumelog_can::tests_common::gen_candump` for the synthetic 100k corpus.

use std::path::PathBuf;
use std::process::{Command, Output};

use lumelog_can::source::{CandumpSource, FrameSource};
use lumelog_can::tests_common::gen_candump;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
}

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lumelog-can")).args(args).output().expect("run cli")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn dbc_summary_counts_messages() {
    let out = cli(&["dbc", "--file", &data("sample.dbc").to_string_lossy(), "--lenient"]);
    assert!(out.status.success(), "dbc --lenient should succeed");
    let text = stdout(&out);
    assert!(text.contains("\"messages\":3"), "3 messages, got: {text}");
    assert!(text.contains("\"errors\":1"), "the bad SG_ line counted, got: {text}");
}

#[test]
fn decode_candump_counts_and_sample() {
    let out = cli(&[
        "decode",
        "--log",
        &data("candump_basic.log").to_string_lossy(),
        "--format",
        "candump",
        "--limit",
        "1",
    ]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("\"decoded\":7,\"skipped\":2"), "stats, got: {text}");
    // First frame is the (ts) 123#DEADBEEF line.
    assert!(text.contains("\"id\":\"0x123\""), "sample frame present, got: {text}");
}

#[test]
fn decode_with_dbc_emits_signal_physical() {
    let out = cli(&[
        "decode",
        "--log",
        &data("cli_intel.log").to_string_lossy(),
        "--format",
        "candump",
        "--dbc",
        &data("bitorder.dbc").to_string_lossy(),
        "--out",
        "jsonl",
    ]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("\"message_name\":\"IntelMsg\""), "msg matched, got: {text}");
    assert!(text.contains("\"name\":\"I_S16\""), "signals decoded, got: {text}");
    // Physical values must equal the cantools golden (Task 3 authority).
    assert!(text.contains("\"raw\":-16723"), "I_S16 raw, got: {text}");
    assert!(text.contains("-8371.5"), "I_S16 physical, got: {text}");
}

#[test]
fn no_args_exits_2() {
    let out = cli(&[]);
    assert_eq!(out.status.code(), Some(2), "usage error -> 2");
}

#[test]
fn bad_format_exits_2() {
    let out = cli(&[
        "decode",
        "--log",
        &data("candump_basic.log").to_string_lossy(),
        "--format",
        "nope",
    ]);
    assert_eq!(out.status.code(), Some(2), "unknown format -> 2");
}

#[test]
fn missing_file_exits_1() {
    let out = cli(&[
        "decode",
        "--log",
        "Z:/definitely/not/here.canlog",
        "--format",
        "candump",
    ]);
    assert_eq!(out.status.code(), Some(1), "missing log -> 1");
}

/// M2 engine correctness face (§12 "10 万帧…解码正确"): a 100k synthetic corpus
/// decodes faithfully — every frame's id/dlc/data matches the generator, nothing
/// is skipped. Also records throughput for the CHANGELOG.
#[test]
fn decodes_100k_frames_faithfully() {
    const N: u64 = 100_000;
    let gen = gen_candump(N, 0xC0FFEE);
    assert_eq!(gen.len() as u64, N);

    let lines = gen.iter().map(|g| (g.line_number, g.text.clone())).collect::<Vec<_>>();
    let mut src = CandumpSource::new(lines.into_iter());

    let t0 = std::time::Instant::now();
    let mut decoded = Vec::with_capacity(N as usize);
    while let Some(f) = src.next_frame() {
        decoded.push(f);
    }
    let elapsed = t0.elapsed();

    let st = src.stats();
    assert_eq!(st.decoded, N, "all frames decoded");
    assert_eq!(st.skipped, 0, "synthetic corpus has no bad lines");
    assert_eq!(decoded.len() as u64, N);

    for (f, g) in decoded.iter().zip(&gen) {
        assert_eq!(f.line_number, g.line_number);
        assert_eq!(f.can_id, g.can_id, "id fidelity @{}", g.line_number);
        assert_eq!(f.dlc as usize, g.data.len(), "dlc fidelity @{}", g.line_number);
        assert_eq!(&f.data[..g.data.len()], &g.data[..], "data fidelity @{}", g.line_number);
    }

    // Throughput note (engine side; UI smoothness is Plan 3 / §12 P4).
    let per_frame = elapsed.as_micros() as f64 / N as f64;
    println!(
        "decoded {N} frames in {:?} ({per_frame:.1} µs/frame)",
        elapsed
    );
}
