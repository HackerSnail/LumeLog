//! Document engine: line indexing and (later) chunked read/decode.

pub mod line_index;

pub use line_index::{build, build_aligned, LineIndex, LineIndexBuilder};
