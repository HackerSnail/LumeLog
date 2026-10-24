//! Document engine: line indexing, chunked read/decode, and the open-file
//! handle with its decoded-chunk cache.

pub mod chunk;
pub mod file;
pub mod line_index;

pub use chunk::{build_chunk, Chunk, CharMap, LogicalLine};
pub use file::DocFile;
pub use line_index::{build, build_aligned, LineIndex, LineIndexBuilder};
