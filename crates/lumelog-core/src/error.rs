use std::io;
use std::time::SystemTime;

/// Core engine error. Shared across document/encoding/search/save.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("file too large (exceeds 4GB u32 index limit or configured size limit)")]
    TooLarge,

    #[error("line index out of range")]
    LineOutOfRange,

    #[error("encoding error: {0}")]
    Encoding(String),

    /// A search pattern could not be used: empty, invalid regex syntax, or not
    /// representable in the target encoding (all Regex/Literal query errors).
    #[error("invalid search pattern: {0}")]
    InvalidPattern(String),

    /// payload = current disk (size, mtime_millis) observed on save attempt
    #[error("disk file changed since open (conflict)")]
    Conflict((u64, SystemTime)),

    /// N characters could not be represented in the target encoding
    #[error("{0} characters are unmappable in the target encoding")]
    Unmappable(usize),

    #[error("target file is locked by another process")]
    Locked,
}
