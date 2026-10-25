//! External-file watch bookkeeping (spec §4.4/A10, M3 子项目 B2). This is the
//! pure half: deciding whether a disk change is real (vs. our own save) and
//! tracking which parent directories need a `notify` watch. The runtime
//! (watcher + 500ms debounce thread + channel) lives below once commands wire it
//! up (Task 2).
#![allow(dead_code)] // consumed by the AppWatch runtime (Task 2)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// What a watched path's on-disk state means for an open document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalKind {
    Changed,
    Removed,
}

/// `disk` is the current on-disk `(size, mtime)`; `snapshot` is the document's
/// recorded pair. Equal -> our own save (or no real change): ignore. Missing ->
/// the file disappeared. Anything else is an external modification.
pub fn classify(
    disk: Option<(u64, SystemTime)>,
    snapshot: (u64, SystemTime),
) -> Option<ExternalKind> {
    match disk {
        None => Some(ExternalKind::Removed),
        Some(meta) if meta == snapshot => None,
        Some(_) => Some(ExternalKind::Changed),
    }
}

/// Parent-directory watch refcounts (one `notify` watch per directory) plus the
/// tracked file paths used to filter events.
#[derive(Debug, Default)]
pub struct WatchRegistry {
    dirs: HashMap<PathBuf, u32>,
    paths: HashSet<PathBuf>,
}

impl WatchRegistry {
    /// Track `path`; returns the directory when it needs a NEW `notify` watch.
    pub fn watch(&mut self, path: &Path) -> Option<PathBuf> {
        if !self.paths.insert(path.to_path_buf()) {
            return None; // already tracked
        }
        let dir = parent_dir(path);
        let n = self.dirs.entry(dir.clone()).or_insert(0);
        *n += 1;
        if *n == 1 {
            Some(dir)
        } else {
            None
        }
    }

    /// Stop tracking `path`; returns the directory whose refcount hit zero (stop
    /// watching it), if any.
    pub fn unwatch(&mut self, path: &Path) -> Option<PathBuf> {
        if !self.paths.remove(path) {
            return None;
        }
        let dir = parent_dir(path);
        match self.dirs.get_mut(&dir) {
            Some(n) if *n > 1 => {
                *n -= 1;
                None
            }
            Some(_) => {
                self.dirs.remove(&dir);
                Some(dir)
            }
            None => None,
        }
    }

    pub fn tracked(&self, path: &Path) -> bool {
        self.paths.contains(path)
    }
}

/// The directory to watch: the file's parent, falling back to `.` for a bare
/// relative filename (real documents always open with an absolute path).
fn parent_dir(path: &Path) -> PathBuf {
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// One external-change notification to the front end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEvent {
    pub path: String,
    pub kind: ExternalKind,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn t(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn classify_equal_ignores() {
        assert_eq!(classify(Some((10, t(5))), (10, t(5))), None);
    }

    #[test]
    fn classify_size_or_mtime_changed() {
        assert_eq!(classify(Some((11, t(5))), (10, t(5))), Some(ExternalKind::Changed));
        assert_eq!(classify(Some((10, t(6))), (10, t(5))), Some(ExternalKind::Changed));
    }

    #[test]
    fn classify_missing_removed() {
        assert_eq!(classify(None, (10, t(5))), Some(ExternalKind::Removed));
    }

    #[test]
    fn registry_same_dir_once() {
        let mut r = WatchRegistry::default();
        assert_eq!(r.watch(Path::new("/logs/a.log")), Some(PathBuf::from("/logs")));
        assert_eq!(r.watch(Path::new("/logs/b.log")), None, "dir already watched");
        assert!(r.tracked(Path::new("/logs/a.log")));
        assert!(!r.tracked(Path::new("/logs/c.log")));
    }

    #[test]
    fn registry_unwatch_refcount() {
        let mut r = WatchRegistry::default();
        r.watch(Path::new("/logs/a.log"));
        r.watch(Path::new("/logs/b.log"));
        assert_eq!(r.unwatch(Path::new("/logs/a.log")), None, "one doc left in dir");
        assert_eq!(r.unwatch(Path::new("/logs/b.log")), Some(PathBuf::from("/logs")));
        assert_eq!(r.unwatch(Path::new("/logs/b.log")), None, "untracked is a no-op");
    }

    #[test]
    fn registry_relative_filename_uses_dot() {
        let mut r = WatchRegistry::default();
        assert_eq!(r.watch(Path::new("a.log")), Some(PathBuf::from(".")));
    }

    #[test]
    fn external_event_serializes_lowercase_kind() {
        let e = ExternalEvent {
            path: "C:/logs/a.log".into(),
            kind: ExternalKind::Changed,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["path"], "C:/logs/a.log");
        assert_eq!(v["kind"], "changed");

        let r: ExternalEvent =
            serde_json::from_str(r#"{"path":"x","kind":"removed"}"#).unwrap();
        assert_eq!(r.kind, ExternalKind::Removed);
    }
}
