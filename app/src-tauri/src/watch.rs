//! External-file watch bookkeeping (spec §4.4/A10, M3 子项目 B2): deciding
//! whether a disk change is real (vs. our own save), tracking parent-directory
//! watches, and the runtime watcher + 500 ms quiet-window debounce thread that
//! turns `notify` events into `ExternalEvent`s for the front end.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager};

use crate::state::AppDocs;

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

/// Parent-directory watch refcounts (one `notify` watch per directory) plus
/// tracked file paths with their own counts (the same file may be opened twice).
#[derive(Debug, Default)]
pub struct WatchRegistry {
    dirs: HashMap<PathBuf, u32>,
    paths: HashMap<PathBuf, u32>,
}

impl WatchRegistry {
    /// Track `path`; returns the directory when it needs a NEW `notify` watch.
    pub fn watch(&mut self, path: &Path) -> Option<PathBuf> {
        let n = self.paths.entry(path.to_path_buf()).or_insert(0);
        *n += 1;
        if *n > 1 {
            return None; // same file already open: only bump its count
        }
        let dir = parent_dir(path);
        let d = self.dirs.entry(dir.clone()).or_insert(0);
        *d += 1;
        if *d == 1 {
            Some(dir)
        } else {
            None
        }
    }

    /// Stop tracking one reference to `path`; returns the directory whose count
    /// hit zero (stop watching it), if any.
    pub fn unwatch(&mut self, path: &Path) -> Option<PathBuf> {
        let dir = parent_dir(path);
        match self.paths.get_mut(path) {
            Some(n) if *n > 1 => {
                *n -= 1;
                return None;
            }
            Some(_) => {
                self.paths.remove(path);
            }
            None => return None,
        }
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
        self.paths.contains_key(path)
    }

    /// Directories currently watched (used to replay them into a new watcher).
    pub fn dirs(&self) -> Vec<PathBuf> {
        self.dirs.keys().cloned().collect()
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

/// Subscription result: a non-`None` reason means watching degraded to
/// save-time-only checks (the app still works, the status bar says so).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchStatus {
    pub degraded: Option<String>,
}

/// Debounce window: emit only after this much quiet.
const DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Inner {
    registry: WatchRegistry,
    channel: Option<Channel<ExternalEvent>>,
    pending: HashSet<PathBuf>,
    degraded: Option<String>,
    stop: bool,
}

/// App-managed runtime: one directory watcher and one debounce thread per app.
/// The watcher lives behind its OWN mutex: `notify`'s `watch`/`unwatch` block
/// until the event thread acknowledges, and that thread runs our callback (which
/// locks `shared`) — so calling notify while holding `shared` could deadlock.
pub struct AppWatch {
    shared: Arc<(Mutex<Inner>, Condvar)>,
    watcher: Mutex<Option<RecommendedWatcher>>,
    started: AtomicBool,
}

impl Default for AppWatch {
    fn default() -> Self {
        Self {
            shared: Arc::new((Mutex::new(Inner::default()), Condvar::new())),
            watcher: Mutex::new(None),
            started: AtomicBool::new(false),
        }
    }
}

impl AppWatch {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.shared.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn set_degraded(&self, reason: String) {
        self.inner().degraded = Some(reason);
    }

    /// Bind the event channel, create the watcher (once), and start the debounce
    /// thread (once). A watcher that cannot be created degrades to save-time
    /// checks only — it never fails this call.
    pub fn start(&self, app: &AppHandle, channel: Channel<ExternalEvent>) -> WatchStatus {
        let dirs = {
            let mut inner = self.inner();
            inner.channel = Some(channel);
            inner.registry.dirs()
        };
        let mut fail: Option<String> = None;
        {
            let mut slot = self.watcher.lock().unwrap_or_else(|p| p.into_inner());
            if slot.is_none() {
                let shared = self.shared.clone();
                let handler = move |res: notify::Result<notify::Event>| {
                    let Ok(ev) = res else { return };
                    let (lock, cv) = &*shared;
                    let mut g = lock.lock().unwrap_or_else(|p| p.into_inner());
                    let mut any = false;
                    for p in ev.paths {
                        if g.registry.tracked(&p) {
                            g.pending.insert(p);
                            any = true;
                        }
                    }
                    drop(g);
                    if any {
                        cv.notify_all();
                    }
                };
                match RecommendedWatcher::new(handler, notify::Config::default()) {
                    Ok(mut w) => {
                        for dir in dirs {
                            if let Err(e) = w.watch(&dir, RecursiveMode::NonRecursive) {
                                fail = Some(format!("文件监听不可用：{e}"));
                            }
                        }
                        *slot = Some(w);
                    }
                    Err(e) => fail = Some(format!("文件监听不可用：{e}")),
                }
            }
        }
        if let Some(e) = fail {
            self.set_degraded(e);
        }
        if !self.started.swap(true, Ordering::SeqCst) {
            let app = app.clone();
            let shared = self.shared.clone();
            std::thread::spawn(move || debounce_loop(app, shared));
        }
        self.status()
    }

    /// Track `path` (a document opened). Safe before `start`: the registry keeps
    /// the directory and `start` replays it into the fresh watcher.
    pub fn watch_path(&self, path: &Path) {
        let dir = {
            let mut inner = self.inner();
            inner.registry.watch(path)
        };
        let Some(dir) = dir else { return };
        let mut fail: Option<String> = None;
        {
            let mut slot = self.watcher.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(w) = slot.as_mut() {
                if let Err(e) = w.watch(&dir, RecursiveMode::NonRecursive) {
                    fail = Some(format!("文件监听不可用：{e}"));
                }
            }
        }
        if let Some(e) = fail {
            self.set_degraded(e);
        }
    }

    /// Stop tracking one `path` reference; stops the directory watch once its
    /// last document is gone.
    pub fn unwatch_path(&self, path: &Path) {
        let dir = {
            let mut inner = self.inner();
            inner.registry.unwatch(path)
        };
        let Some(dir) = dir else { return };
        let mut slot = self.watcher.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(w) = slot.as_mut() {
            let _ = w.unwatch(&dir);
        }
    }

    /// Save-As moved a document: unwatch the old path, watch the new one.
    pub fn retarget(&self, old: &Path, new: &Path) {
        if old == new {
            return;
        }
        self.unwatch_path(old);
        self.watch_path(new);
    }

    /// Current degradation state (surfaced via `watch_status`).
    pub fn status(&self) -> WatchStatus {
        WatchStatus {
            degraded: self.inner().degraded.clone(),
        }
    }
}

/// Wait for pending events, then for 500 ms of silence, then classify each path
/// against the open docs' `(size, mtime)` and emit one event per path.
fn debounce_loop(app: AppHandle, shared: Arc<(Mutex<Inner>, Condvar)>) {
    let (lock, cv) = &*shared;
    loop {
        let (batch, channel) = {
            let mut inner = lock.lock().unwrap_or_else(|p| p.into_inner());
            while inner.pending.is_empty() && !inner.stop {
                inner = cv.wait(inner).unwrap_or_else(|p| p.into_inner());
            }
            if inner.stop {
                return;
            }
            // Re-armed by every new event: only a full quiet window flushes.
            loop {
                let (guard, timeout) = cv
                    .wait_timeout(inner, DEBOUNCE)
                    .unwrap_or_else(|p| p.into_inner());
                inner = guard;
                if inner.stop {
                    return;
                }
                if timeout.timed_out() {
                    break;
                }
            }
            let batch: Vec<PathBuf> = inner.pending.drain().collect();
            (batch, inner.channel.clone())
        };
        let Some(ch) = channel else { continue };
        let state = app.state::<AppDocs>();
        for path in batch {
            let snaps = state.snapshot_for_path(&path);
            if snaps.is_empty() {
                continue;
            }
            let mut kind: Option<ExternalKind> = None;
            for (_id, snap, disk) in snaps {
                match classify(disk, snap) {
                    Some(ExternalKind::Removed) => {
                        kind = Some(ExternalKind::Removed);
                        break;
                    }
                    Some(_) => {
                        kind.get_or_insert(ExternalKind::Changed);
                    }
                    None => {}
                }
            }
            if let Some(kind) = kind {
                let _ = ch.send(ExternalEvent {
                    path: path.to_string_lossy().to_string(),
                    kind,
                });
            }
        }
    }
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
    fn registry_same_path_keeps_watch_until_last_close() {
        let mut r = WatchRegistry::default();
        let p = Path::new("/logs/a.log");
        assert_eq!(r.watch(p), Some(PathBuf::from("/logs")));
        assert_eq!(r.watch(p), None, "second open of the same file only bumps count");
        assert_eq!(r.unwatch(p), None, "one reference left");
        assert!(r.tracked(p));
        assert_eq!(r.unwatch(p), Some(PathBuf::from("/logs")), "last reference stops the dir");
        assert!(!r.tracked(p));
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

    #[test]
    fn watch_status_starts_not_degraded() {
        let w = AppWatch::default();
        assert_eq!(w.status().degraded, None);
    }
}
