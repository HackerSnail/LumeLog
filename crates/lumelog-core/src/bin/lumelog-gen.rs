//! `lumelog-gen` — deterministic synthetic-log generator for the M1
//! performance checklist (docs/test-checklist.md §1). Same generator the
//! benches use (`tests_common::gen_log`), so P3/P5 numbers are comparable
//! with the P4 bench output.
//!
//! ```text
//! lumelog-gen OUT_PATH SIZE_MB
//! ```
//!
//! Writes ~`SIZE_MB` MiB of ASCII HIL bench lines (fixed seed) to OUT_PATH and
//! prints a JSON summary. Exit codes: 0 ok, 2 usage, 1 I/O.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.len() != 2 {
        eprintln!("usage: lumelog-gen OUT_PATH SIZE_MB");
        return ExitCode::from(2);
    }
    let size_mb: usize = match argv[1].parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("lumelog-gen: SIZE_MB must be a number, got '{}'", argv[1]);
            return ExitCode::from(2);
        }
    };
    let path = Path::new(&argv[0]);
    let bytes = lumelog_core::tests_common::gen_log(size_mb);
    let mut f = match std::fs::File::create(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("lumelog-gen: {}: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    match f.write_all(&bytes).and_then(|()| f.flush()) {
        Ok(()) => {
            println!("{{\"path\":\"{}\",\"bytes\":{}}}", argv[0], bytes.len());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("lumelog-gen: write {}: {e}", path.display());
            ExitCode::from(1)
        }
    }
}
