//! M0 CLI smoke test for the `lumelog` binary (Task 7). Runs the compiled bin
//! via `CARGO_BIN_EXE_lumelog` (set by cargo for this package's bins) against a
//! committed GBK sample, and pins the summary JSON + exit codes.

use lumelog_core::encoding::{encode_all, EncodingLabel};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn sample_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/sample_gbk.log")
}

/// 20 GBK bench lines with NO trailing newline (so `lines == 20`), written to
/// `tests/data/sample_gbk.log` when it is missing or stale. Returns the bytes.
fn ensure_sample() -> Vec<u8> {
    let mut text = String::new();
    for i in 0..20 {
        if i > 0 {
            text.push('\n');
        }
        text.push_str(&format!(
            "2026-10-24 12:00:{i:02} INFO 台架 HIL 测试 记录 完成 val={i}"
        ));
    }
    let bytes = encode_all(EncodingLabel::Gbk, &text).bytes;
    if fs::read(sample_path()).map(|b| b != bytes).unwrap_or(true) {
        fs::create_dir_all(sample_path().parent().unwrap()).unwrap();
        fs::write(sample_path(), &bytes).unwrap();
    }
    bytes
}

/// A `lumelog` invocation rooted at the crate dir so relative paths resolve.
fn lumelog() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lumelog"));
    cmd.current_dir(env!("CARGO_MANIFEST_DIR"));
    cmd
}

#[test]
fn open_prints_summary_json() {
    let bytes = ensure_sample();
    let out = lumelog()
        .args(["open", "--file", "tests/data/sample_gbk.log"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "open failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        stdout.contains(&format!("\"size\":{}", bytes.len())),
        "size mismatch: {stdout}"
    );
    assert!(stdout.contains("\"lines\":20"), "lines: {stdout}");
    assert!(
        stdout.contains("\"encoding\":\"GBK\"") || stdout.contains("\"encoding\":\"GB18030\""),
        "encoding: {stdout}"
    );
    assert!(stdout.contains("\"eol\":\"lf\""), "eol should be lf: {stdout}");
}

#[test]
fn open_line_count_flag_prints_only_count() {
    ensure_sample();
    let out = lumelog()
        .args(["open", "--file", "tests/data/sample_gbk.log", "--line-count"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "20");
}

#[test]
fn search_counts_literal_gbk_hits() {
    ensure_sample();
    let out = lumelog()
        .args([
            "search",
            "--file",
            "tests/data/sample_gbk.log",
            "--pattern",
            "台架",
            "--encoding",
            "GBK",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"matches\":20"), "expected 20 hits: {stdout}");
    // Each hit line carries CodeMirror-ready coordinates.
    assert!(stdout.contains("\"byte_in_line\":") && stdout.contains("\"snippet\":"));
}

#[test]
fn save_roundtrips_gbk_from_stdin() {
    ensure_sample();
    let tmp = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/_save_roundtrip.log");
    fs::copy(sample_path(), &tmp).unwrap();
    let mut child = lumelog()
        .args(["save", "--file", tmp.to_str().unwrap(), "--encoding", "GBK"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Replace the document with a short Chinese line; save re-encodes to GBK.
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all("新的一行\n".as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "save failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let written = fs::read(&tmp).unwrap();
    assert_eq!(
        written,
        encode_all(EncodingLabel::Gbk, "新的一行\n").bytes,
        "saved bytes must be the GBK re-encoding of stdin"
    );
    fs::remove_file(&tmp).ok();
}

#[test]
fn no_args_exits_with_code_2() {
    let out = lumelog().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!out.stderr.is_empty(), "usage should print to stderr");
}

#[test]
fn unknown_subcommand_exits_2() {
    let out = lumelog().arg("frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn missing_file_is_error_code_1() {
    let out = lumelog()
        .args(["open", "--file", "tests/data/does_not_exist.log"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

/// A missing required flag is a *usage* error (exit 2), not a runtime error
/// (exit 1) — the M0 exit-code contract classifies it as usage.
#[test]
fn missing_required_flag_is_usage_exit_2() {
    let out = lumelog().arg("open").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("missing --file"), "stderr: {err}");
}

#[test]
fn bad_flag_value_is_usage_exit_2() {
    ensure_sample();
    let out = lumelog()
        .args(["open", "--file", "tests/data/sample_gbk.log", "--head", "abc"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--head must be a number"));
}

/// Manual M0 P2/P3 smoke (opt-in): builds a ~1 GiB log and times the CLI
/// `open` path end to end — the whole in-memory line index is built before the
/// summary JSON returns, so the wall time is the P3 index-build reference. Run
/// with `cargo test -p lumelog-core --test cli_smoke -- --ignored --nocapture`.
#[test]
#[ignore = "manual 1 GiB open timing; slow + large temp file"]
fn open_1gb_wall_time() {
    use std::time::Instant;
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/_bench_1gb.log");
    let bytes = lumelog_core::tests_common::gen_log(1024); // deterministic UTF-8, ~1 GiB
    let size = bytes.len();
    fs::write(&path, &bytes).unwrap();
    drop(bytes);
    let start = Instant::now();
    let out = lumelog()
        .args(["open", "--file", path.to_str().unwrap()])
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        out.status.success(),
        "open failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    println!(
        "1GiB open: {} bytes, summary={}, wall={elapsed:?}",
        size,
        String::from_utf8_lossy(&out.stdout).trim()
    );
    fs::remove_file(&path).ok();
}
