//! Document engine: line indexing, chunked read/decode, and the open-file
//! handle with its decoded-chunk cache.

pub mod chunk;
pub mod file;
pub mod line_index;
pub mod save;

pub use chunk::{build_chunk, Chunk, CharMap, LogicalLine};
pub use file::{build_index_with_progress, validate_open_path, DocFile};
pub use line_index::{build, build_aligned, LineIndex, LineIndexBuilder};
pub use save::{detect_eol, save_text, EolStyle, OverwritePolicy, SaveOptions, SaveReport};
