//! Launch-argument handling + single-instance handoff (Plan 43). Files passed
//! on the command line (double-click "Open with", association) are collected
//! once at process start; a second instance's argv arrives through the
//! single-instance plugin callback and is forwarded to the front end.

use std::sync::OnceLock;

static LAUNCH_ARGS: OnceLock<Vec<String>> = OnceLock::new();

/// Files from `argv` (skip the program path and `-`-prefixed flags).
pub fn from_argv(argv: &[String]) -> Vec<String> {
    argv.iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect()
}

/// Capture the initial command line before anything else runs.
pub fn collect() {
    let _ = LAUNCH_ARGS.set(from_argv(&std::env::args().collect::<Vec<String>>()));
}

/// Initial launch files for the front end (empty when none / already drained).
#[tauri::command]
pub fn launch_args() -> Vec<String> {
    LAUNCH_ARGS.get().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_argv_skips_program_and_flags() {
        let argv = vec![
            "lumelog.exe".to_string(),
            "C:\\logs\\a.log".to_string(),
            "--flag".to_string(),
            "-x".to_string(),
            "b.log".to_string(),
        ];
        assert_eq!(from_argv(&argv), vec!["C:\\logs\\a.log".to_string(), "b.log".to_string()]);
        assert!(from_argv(&["lumelog.exe".to_string()]).is_empty());
    }
}
