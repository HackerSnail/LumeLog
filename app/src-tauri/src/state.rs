//! Shared app state behind the IPC commands. `DocFile` is `Send`, so each open
//! document lives in an `Arc<Mutex<..>>` keyed by an id the front end passes
//! back on every call. In-flight searches register an cancel flag so
//! `search_cancel(request_id)` can stop a scan cooperatively.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use lumelog_can::dbc::Database;
use lumelog_core::doc::DocFile;

#[derive(Default)]
pub struct AppDocs {
    docs: Mutex<HashMap<u64, Arc<Mutex<DocFile>>>>,
    next_doc_id: AtomicU64,
    cancels: Mutex<HashMap<u64, Arc<AtomicBool>>>,
    next_req_id: AtomicU64,
    /// Imported DBC databases, keyed by an id the front end passes back on every
    /// `can_parse`/`can_decode_snippet` call.
    dbc: Mutex<HashMap<u64, Arc<Database>>>,
    next_dbc_id: AtomicU64,
}

impl AppDocs {
    fn next_id(counter: &AtomicU64) -> u64 {
        counter.fetch_add(1, Ordering::Relaxed)
    }

    /// Register a freshly opened document, returning its id.
    pub fn insert(&self, doc: DocFile) -> u64 {
        let id = Self::next_id(&self.next_doc_id);
        self.docs
            .lock()
            .unwrap()
            .insert(id, Arc::new(Mutex::new(doc)));
        id
    }

    pub fn get(&self, id: u64) -> Option<Arc<Mutex<DocFile>>> {
        self.docs.lock().unwrap().get(&id).cloned()
    }

    pub fn ids(&self) -> Vec<u64> {
        self.docs.lock().unwrap().keys().copied().collect()
    }

    /// Drop a document; returns whether the id was present. The last `Arc` goes
    /// away here, so the `DocFile` (file handle + line index + LRU cache) is
    /// released — repeated open/close no longer leaks documents.
    pub fn remove(&self, id: u64) -> bool {
        self.docs.lock().unwrap().remove(&id).is_some()
    }

    /// `(doc_id, (size, mtime))` for every open doc whose path equals `path`.
    /// The external-watch debouncer compares these against the disk to tell our
    /// own saves from real external changes.
    pub fn snapshot_for_path(&self, path: &Path) -> Vec<(u64, (u64, std::time::SystemTime))> {
        let docs: Vec<(u64, Arc<Mutex<DocFile>>)> = self
            .docs
            .lock()
            .unwrap()
            .iter()
            .map(|(id, arc)| (*id, arc.clone()))
            .collect();
        docs.into_iter()
            .filter_map(|(id, arc)| {
                let d = arc.lock().unwrap_or_else(|p| p.into_inner());
                if d.path() == path {
                    Some((id, d.disk_meta()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Replace a document's `DocFile` in place, keeping its id and `Arc` so any
    /// in-flight reader (`DocLines`/export) immediately sees the fresh file.
    /// Returns false for an unknown id.
    pub fn replace(&self, id: u64, doc: DocFile) -> bool {
        match self.get(id) {
            Some(arc) => {
                *arc.lock().unwrap_or_else(|p| p.into_inner()) = doc;
                true
            }
            None => false,
        }
    }

    /// Begin a search: mint a request id and its (shared) cancel flag.
    pub fn begin_search(&self) -> (u64, Arc<AtomicBool>) {
        let id = Self::next_id(&self.next_req_id);
        let flag = Arc::new(AtomicBool::new(false));
        self.cancels.lock().unwrap().insert(id, flag.clone());
        (id, flag)
    }

    /// Signal an in-flight search to stop. Returns whether the id was known.
    pub fn cancel(&self, id: u64) -> bool {
        if let Some(flag) = self.cancels.lock().unwrap().get(&id) {
            flag.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Drop a finished search's cancel flag.
    pub fn end_search(&self, id: u64) {
        self.cancels.lock().unwrap().remove(&id);
    }

    /// Register a freshly imported DBC database, returning its id.
    pub fn insert_dbc(&self, db: Arc<Database>) -> u64 {
        let id = Self::next_id(&self.next_dbc_id);
        self.dbc.lock().unwrap().insert(id, db);
        id
    }

    pub fn get_dbc(&self, id: u64) -> Option<Arc<Database>> {
        self.dbc.lock().unwrap().get(&id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbc_store_roundtrip() {
        let s = AppDocs::default();
        let a = s.insert_dbc(Arc::new(Database::default()));
        let b = s.insert_dbc(Arc::new(Database::default()));
        assert_ne!(a, b, "each import mints a distinct id");
        assert!(s.get_dbc(a).is_some());
        assert!(s.get_dbc(b).is_some());
        assert!(s.get_dbc(9_999).is_none(), "unknown id misses");
    }

    #[test]
    fn snapshot_for_path_matches_open_docs() {
        let s = AppDocs::default();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut f, b"one\ntwo\n").unwrap();
        let path = f.path().to_path_buf();
        let a = s.insert(DocFile::open(&path, 1 << 20).unwrap());
        let b = s.insert(DocFile::open(&path, 1 << 20).unwrap());
        let mut got: Vec<u64> = s
            .snapshot_for_path(&path)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        got.sort_unstable();
        let mut want = vec![a, b];
        want.sort_unstable();
        assert_eq!(got, want);
        assert!(s.snapshot_for_path(Path::new("C:/nope/x.log")).is_empty());
    }

    #[test]
    fn replace_swaps_in_place() {
        let s = AppDocs::default();
        let mut a = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut a, b"one\n").unwrap();
        let mut b = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut b, b"x\ny\nz").unwrap(); // no trailing \n -> 3 lines

        let id = s.insert(DocFile::open(a.path(), 1 << 20).unwrap());
        let fresh = DocFile::open(b.path(), 1 << 20).unwrap();
        assert!(s.replace(id, fresh));
        let lines = s.get(id).unwrap().lock().unwrap().line_count();
        assert_eq!(lines, 3, "same id now serves the replacement doc");
        assert!(!s.replace(9_999, DocFile::open(b.path(), 1 << 20).unwrap()));
    }
}
