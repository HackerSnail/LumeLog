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

/// Result of the *pre-open* checks the app runs before handing a path to
/// `DocFile::open` (spec §9). Kept separate from `CoreError` so the IPC layer
/// can map each case to a distinct, stable error code (`not_found`,
/// `is_directory`, `permission_denied`, `too_large`) rather than a generic io
/// failure. `PartialEq` so the pure validator can be unit-tested by equality.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenReject {
    #[error("path does not exist")]
    NotFound,

    #[error("path is a directory")]
    IsDirectory,

    #[error("permission denied")]
    PermissionDenied,

    /// `0` = file size, `1` = configured limit.
    #[error("file too large ({0} bytes > {1} byte limit)")]
    TooLarge(u64, u64),
}
