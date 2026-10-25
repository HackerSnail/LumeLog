//! LumeLog CAN engine: DBC subset parsing, Intel/Motorola signal decode, and
//! streaming CAN-log sources (candump / Vector ASC / generic regex). Pure Rust,
//! no Tauri dependency — reusable from the decode CLI (Task 7) and the app shell.
//!
//! Plan 2 grows this crate task by task: `decode` is added in Task 3, `source` in
//! Task 4. Only modules that exist are declared here.

pub mod blf;
pub mod dbc;
pub mod decode;
pub mod error;
pub mod frame;
pub mod source;
pub mod tests_common;

pub use blf::BlfSource;

#[cfg(test)]
mod tests {
    use super::error::CanError;

    #[test]
    fn crate_links() {}

    #[test]
    fn ipc_codes_are_stable() {
        // Pins the spec §9 IPC error-code contract; the frontend renders Chinese
        // text keyed off these exact strings, so a rename is a breaking change.
        let cases = [
            (CanError::Io(std::io::Error::other("x")), "io"),
            (
                CanError::DbcSyntax {
                    line: 1,
                    context: String::new(),
                    kind: String::new(),
                },
                "dbc_syntax",
            ),
            (CanError::BadRegex("r".into()), "bad_regex"),
            (CanError::RegexCompile("r".into()), "regex_compile"),
            (CanError::SourceFormat("s".into()), "source_format"),
        ];
        for (err, code) in cases {
            assert_eq!(err.ipc_code(), code);
        }
    }
}
