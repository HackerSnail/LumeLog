//! LumeLog core engine: line indexing, encoding, chunked decode, search, save.
//! Pure Rust, no Tauri dependency — reusable from CLI and the app shell.

pub mod doc;
pub mod encoding;
pub mod error;
pub mod export;
pub mod filter;
pub mod search;

// Deterministic log generator, shared by unit tests, integration tests and
// benches. Kept public (not `cfg(test)`) because a criterion bench target
// compiles this crate as an external dependency, where `cfg(test)` is off.
pub mod tests_common;

#[cfg(test)]
#[test]
fn crate_links() {
    // Scaffold smoke test proving the crate compiles and links (Task 1).
    let bytes = crate::tests_common::gen_log(1);
    assert!(!bytes.is_empty());
}
