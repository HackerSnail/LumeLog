//! Recent-files store (M1 deferred debt, spec
//! `docs/specs/2026-10-30-drop-recent-design.md`): `$APP_CONFIG/LumeLog/recent.json`
//! holds up to `RECENT_CAP` paths, most recent first. `doc_open` records a
//! successful open; the palette lists them. The file is best-effort state —
//! a missing or corrupted file just means an empty list, never an error.

use std::path::{Path, PathBuf};

use crate::dto::IpcError;

pub(crate) const RECENT_CAP: usize = 20;

pub(crate) fn recent_path(app: &tauri::AppHandle) -> Result<PathBuf, IpcError> {
    let config = crate::commands::config_path(app)?;
    Ok(config
        .parent()
        .map(|p| p.join("recent.json"))
        .unwrap_or_else(|| PathBuf::from("recent.json")))
}

/// Read the list; a missing or corrupted file is an empty list (best-effort).
pub(crate) fn load_recent(path: &Path) -> Vec<String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    match serde_json::from_str::<Vec<String>>(&text) {
        Ok(v) => v.into_iter().take(RECENT_CAP).collect(),
        Err(_) => Vec::new(),
    }
}

/// Move `opened` to the front (dedup) and persist atomically. The file is
/// advisory state: callers ignore the error.
pub(crate) fn record_recent(store: &Path, opened: &str) -> std::io::Result<()> {
    let mut list = load_recent(store);
    list.retain(|p| p != opened);
    list.insert(0, opened.to_string());
    list.truncate(RECENT_CAP);
    let body = serde_json::to_string(&list).unwrap_or_else(|_| "[]".into());
    if let Some(parent) = store.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = store.with_extension(format!(
        "json.tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, store)
}

/// Empty the list (the file stays, holding `[]`).
pub(crate) fn clear_recent(store: &Path) -> std::io::Result<()> {
    if let Some(parent) = store.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(store, "[]")
}

/// IPC entry: the recorded paths that still exist, most recent first.
#[tauri::command]
pub fn recent_list(app: tauri::AppHandle) -> Result<Vec<String>, IpcError> {
    let path = recent_path(&app)?;
    Ok(load_recent(&path)
        .into_iter()
        .filter(|p| std::fs::metadata(p).is_ok())
        .collect())
}

#[tauri::command]
pub fn recent_clear(app: tauri::AppHandle) -> Result<(), IpcError> {
    let path = recent_path(&app)?;
    clear_recent(&path).map_err(|e| IpcError::new("io", format!("清除最近文件失败：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumelog-recent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_and_corrupted_files_are_empty() {
        let dir = temp_dir("empty");
        assert!(load_recent(&dir.join("nope.json")).is_empty());
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{ not json").unwrap();
        assert!(load_recent(&bad).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn record_dedups_to_front_and_caps() {
        let dir = temp_dir("dedup");
        let store = dir.join("recent.json");
        for i in 0..RECENT_CAP + 3 {
            record_recent(&store, &format!("f{i}.log")).unwrap();
        }
        let list = load_recent(&store);
        assert_eq!(list.len(), RECENT_CAP);
        assert_eq!(list[0], format!("f{}.log", RECENT_CAP + 2));
        assert!(!list.contains(&"f0.log".to_string()));

        // re-open an old entry: it moves to the front, no duplicate
        record_recent(&store, "f5.log").unwrap();
        let list = load_recent(&store);
        assert_eq!(list[0], "f5.log");
        assert_eq!(list.iter().filter(|p| *p == "f5.log").count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clear_then_record_roundtrip() {
        let dir = temp_dir("clear");
        let store = dir.join("recent.json");
        record_recent(&store, "a.log").unwrap();
        clear_recent(&store).unwrap();
        assert!(load_recent(&store).is_empty());
        record_recent(&store, "b.log").unwrap();
        assert_eq!(load_recent(&store), vec!["b.log".to_string()]);
        // atomic write leaves no tmp siblings behind
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn persisted_shape_is_a_plain_string_array() {
        let dir = temp_dir("shape");
        let store = dir.join("recent.json");
        record_recent(&store, "x.log").unwrap();
        let v: Vec<String> = serde_json::from_str(&std::fs::read_to_string(&store).unwrap()).unwrap();
        assert_eq!(v, vec!["x.log".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
