//! LumeLog core engine: line indexing, encoding, chunked decode, search, save.
//! Pure Rust, no Tauri dependency — reusable from CLI and the app shell.

pub mod doc;
pub mod encoding;
pub mod error;

#[cfg(test)]
mod tests_common;

#[cfg(test)]
#[test]
fn crate_links() {
    // Scaffold smoke test proving the crate compiles and links (Task 1).
    let bytes = crate::tests_common::gen_log(1);
    assert!(!bytes.is_empty());
}
