//! CAN module errors. Each variant maps to the IPC error contract (spec §9):
//! `ipc_code()` yields the stable lowercase-snake code the frontend keys its
//! message off. Kept in sync with `lumelog-core`'s IpcError style.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CanError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A DBC line failed to parse. `line` is 1-based; `context` is the offending
    /// source text; `kind` names the failure (e.g. `"bad SG_"`).
    #[error("dbc syntax error at line {line} ({kind}): {context}")]
    DbcSyntax {
        line: usize,
        context: String,
        kind: String,
    },

    /// A user-supplied regex is structurally invalid for our purposes (e.g.
    /// missing a required named group).
    #[error("invalid regex: {0}")]
    BadRegex(String),

    /// A user-supplied regex failed to compile.
    #[error("regex compile failed: {0}")]
    RegexCompile(String),

    /// A log line could not be decoded into a frame by the active source.
    #[error("source format error: {0}")]
    SourceFormat(String),
}

impl CanError {
    /// Stable IPC error code (spec §9). Pinned by `ipc_codes_are_stable` —
    /// changing one here MUST change the frontend's code->text table in tandem.
    pub fn ipc_code(&self) -> &'static str {
        match self {
            CanError::Io(_) => "io",
            CanError::DbcSyntax { .. } => "dbc_syntax",
            CanError::BadRegex(_) => "bad_regex",
            CanError::RegexCompile(_) => "regex_compile",
            CanError::SourceFormat(_) => "source_format",
        }
    }
}
