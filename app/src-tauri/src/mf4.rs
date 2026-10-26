//! MF4 (ASAM MDF4) measurement sessions — Roadmap-4 increment 1. This module is
//! the only place that touches `falcon_mdf`; everything above it sees our own
//! DTOs, so the library can be swapped without touching the UI contract.
//!
//! An MF4 file is not a text document: it opens as a *session* (id + handle +
//! cached channel list) and renders in its own dock. Sample data is read lazily
//! and windowed, so no call here materialises a whole signal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use falcon_mdf::{Channel, Mf4File};
use serde::Serialize;

use crate::dto::IpcError;

/// How many measurement files may stay open at once (mmap handles + index).
pub(crate) const MAX_SESSIONS: usize = 4;
/// Column cap for one table page / export run.
#[allow(dead_code)] // consumed by `records`/export (Task 2/3)
pub(crate) const MAX_COLUMNS: usize = 32;
/// Hard cap on rows returned by one `records` call.
#[allow(dead_code)] // consumed by `records` (Task 2)
pub(crate) const MAX_LIMIT: u32 = 1000;
/// Page size ceiling for the channel list.
const MAX_CHANNEL_PAGE: u32 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mf4Kind {
    Number,
    Text,
    Bool,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4ChannelJson {
    /// Session-local stable id, used by `records` and export.
    pub index: u32,
    pub name: String,
    pub unit: String,
    pub kind: Mf4Kind,
    /// Master (time) channel of its group.
    pub master: bool,
    pub group: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4ChannelsPage {
    pub items: Vec<Mf4ChannelJson>,
    pub total: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4InfoJson {
    pub id: u64,
    pub version: String,
    pub path: String,
    pub channel_count: u32,
    pub data_groups: u32,
    pub comment: String,
}

pub(crate) struct Mf4Session {
    /// Read by the windowed record/export paths (Task 2/3).
    #[allow(dead_code)]
    pub file: Arc<Mutex<Mf4File>>,
    #[allow(dead_code)]
    pub path: PathBuf,
    /// Channel metadata in file order, paired with the handle used to read it.
    pub channels: Vec<(Mf4ChannelJson, Channel)>,
}

/// Managed state: open measurement sessions keyed by id.
#[derive(Default)]
pub struct Mf4Sessions {
    inner: Mutex<HashMap<u64, Arc<Mutex<Mf4Session>>>>,
    next_id: AtomicU64,
}

impl Mf4Sessions {
    fn insert(&self, session: Mf4Session) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        self.locked().insert(id, Arc::new(Mutex::new(session)));
        id
    }

    pub(crate) fn get(&self, id: u64) -> Option<Arc<Mutex<Mf4Session>>> {
        self.locked().get(&id).cloned()
    }

    pub(crate) fn remove(&self, id: u64) {
        self.locked().remove(&id);
    }

    pub(crate) fn len(&self) -> usize {
        self.locked().len()
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<Mutex<Mf4Session>>>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Best-effort mapping of MDF data types onto the cell kinds the UI renders.
/// Matching the debug spelling keeps our types free of the library's variants;
/// MDF stores most channels as numbers, so anything not clearly textual/boolean
/// is treated as a number and a genuinely odd type degrades to `Unknown`.
fn kind_of(ch: &Channel) -> Mf4Kind {
    let raw = format!("{:?}", ch.data_type).to_ascii_lowercase();
    if raw.contains("string") || raw.contains("byte") || raw.contains("text") {
        Mf4Kind::Text
    } else if raw.contains("bool") {
        Mf4Kind::Bool
    } else if raw.contains("int")
        || raw.contains("uint")
        || raw.contains("float")
        || raw.contains("real")
    {
        Mf4Kind::Number
    } else {
        Mf4Kind::Unknown
    }
}

/// Classify a library error into our stable IPC codes. Anything that is not a
/// genuine I/O failure at open time is structural, i.e. "not an MF4 file".
fn classify(err: falcon_mdf::Mf4Error) -> IpcError {
    let msg = err.to_string();
    let low = msg.to_ascii_lowercase();
    if low.contains("encrypt") {
        return IpcError::new("mf4_encrypted", format!("文件已加密：{msg}"));
    }
    if low.contains("unsupported") || low.contains("version") {
        return IpcError::new("mf4_unsupported", format!("暂不支持的 MF4 特性：{msg}"));
    }
    let io_like = low.contains("os error")
        || low.contains("permission")
        || low.contains("denied")
        || low.contains("not found")
        || low.contains("eof");
    if io_like {
        return IpcError::new("io", format!("MF4 读取失败：{msg}"));
    }
    IpcError::new("mf4_not_mf4", format!("不是有效的 MF4 文件：{msg}"))
}

fn unknown_session(id: u64) -> IpcError {
    IpcError::new("unknown_mf4", format!("没有 id 为 {id} 的测量会话"))
}

pub(crate) fn open_session(sessions: &Mf4Sessions, path: &Path) -> Result<Mf4InfoJson, IpcError> {
    if sessions.len() >= MAX_SESSIONS {
        return Err(IpcError::new(
            "invalid_argument",
            format!("最多同时打开 {MAX_SESSIONS} 个测量文件，请先关闭一个"),
        ));
    }
    let file = Mf4File::open(path).map_err(classify)?;
    let info = Mf4InfoJson {
        id: 0, // replaced once the session id is known
        version: format!("{}", file.version()),
        path: path.to_string_lossy().into_owned(),
        channel_count: file.channel_count() as u32,
        data_groups: file.data_group_count() as u32,
        comment: file.comment().to_string(),
    };
    let channels: Vec<(Mf4ChannelJson, Channel)> = file
        .channels()
        .enumerate()
        .map(|(i, ch)| {
            let meta = Mf4ChannelJson {
                index: i as u32,
                name: ch.name.clone(),
                unit: ch.unit.clone(),
                kind: kind_of(ch),
                master: ch.is_master(),
                group: ch.channel_group_index as u32,
            };
            (meta, ch.clone())
        })
        .collect();
    let id = sessions.insert(Mf4Session {
        file: Arc::new(Mutex::new(file)),
        path: path.to_path_buf(),
        channels,
    });
    Ok(Mf4InfoJson { id, ..info })
}

pub(crate) fn close_session(sessions: &Mf4Sessions, id: u64) {
    sessions.remove(id); // idempotent by design
}

pub(crate) fn list_channels(
    sessions: &Mf4Sessions,
    id: u64,
    query: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Mf4ChannelsPage, IpcError> {
    let session = sessions.get(id).ok_or_else(|| unknown_session(id))?;
    let guard = session.lock().unwrap_or_else(|p| p.into_inner());
    let needle = query.unwrap_or("").trim().to_ascii_lowercase();
    let mut matched: Vec<&Mf4ChannelJson> = guard
        .channels
        .iter()
        .map(|(m, _)| m)
        .filter(|m| needle.is_empty() || m.name.to_ascii_lowercase().contains(&needle))
        .collect();
    matched.sort_by(|a, b| a.name.cmp(&b.name).then(a.index.cmp(&b.index)));
    let total = matched.len() as u32;
    let items = matched
        .into_iter()
        .skip(offset as usize)
        .take(limit.clamp(1, MAX_CHANNEL_PAGE) as usize)
        .cloned()
        .collect();
    Ok(Mf4ChannelsPage { items, total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use falcon_mdf::Mf4Writer;

    /// Fresh temp dir + a small written MF4 (master time channel + 2 channels).
    fn fixture(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "lumelog-mf4-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.mf4");
        let mut w = Mf4Writer::with_start_time_ns(1_700_000_000_000_000_000);
        {
            let g = w.add_group(&[0.0, 1.0, 2.0, 3.0]).unwrap();
            g.add_channel("Speed", "km/h", &[10.0, 20.0, 30.0, 40.0])
                .unwrap();
            g.add_channel("EngineTemp", "degC", &[1.5, 2.0, 2.5, 3.0])
                .unwrap();
        }
        w.write_to_file(&path).unwrap();
        (dir, path)
    }

    #[test]
    fn open_reports_counts_and_channels() {
        let (dir, path) = fixture("open");
        let sessions = Mf4Sessions::default();
        let info = open_session(&sessions, &path).unwrap();
        assert!(info.id > 0);
        assert!(!info.version.is_empty());
        assert_eq!(info.data_groups, 1);
        // The writer stores a master time channel alongside the two signals.
        assert!(
            info.channel_count >= 2 && (info.channel_count as usize) <= 4,
            "unexpected channel count {}",
            info.channel_count
        );
        let page = list_channels(&sessions, info.id, None, 100, 0).unwrap();
        assert_eq!(page.total, info.channel_count);
        assert!(page.items.iter().any(|c| c.name == "Speed"));
        assert!(page.items.iter().any(|c| c.master), "time axis present");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn channels_query_is_case_insensitive_and_paged() {
        let (dir, path) = fixture("query");
        let sessions = Mf4Sessions::default();
        let info = open_session(&sessions, &path).unwrap();
        let hit = list_channels(&sessions, info.id, Some("  SPEED "), 100, 0).unwrap();
        assert_eq!(hit.items.len(), 1);
        assert_eq!(hit.total, 1);
        let miss = list_channels(&sessions, info.id, Some("zzz"), 100, 0).unwrap();
        assert!(miss.items.is_empty() && miss.total == 0);
        let paged = list_channels(&sessions, info.id, None, 1, 99).unwrap();
        assert!(paged.items.is_empty(), "offset past the end is an empty page");
        assert_eq!(paged.total, info.channel_count);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn open_rejects_non_mf4_and_respects_session_limit() {
        let (dir, _) = fixture("limit");
        let bogus = dir.join("nope.mf4");
        std::fs::write(&bogus, b"not an mdf file at all").unwrap();
        let sessions = Mf4Sessions::default();
        let err = open_session(&sessions, &bogus).unwrap_err();
        assert_eq!(err.code, "mf4_not_mf4");

        for i in 0..MAX_SESSIONS {
            let p = dir.join(format!("s{i}.mf4"));
            std::fs::copy(bogus.parent().unwrap().join("t.mf4"), &p).unwrap();
            open_session(&sessions, &p).unwrap();
        }
        let extra = dir.join("t.mf4");
        assert_eq!(
            open_session(&sessions, &extra).unwrap_err().code,
            "invalid_argument"
        );
        assert_eq!(sessions.len(), MAX_SESSIONS);
        close_session(&sessions, 1);
        close_session(&sessions, 1); // idempotent
        assert_eq!(sessions.len(), MAX_SESSIONS - 1);
        assert_eq!(
            list_channels(&sessions, 999, None, 10, 0).unwrap_err().code,
            "unknown_mf4"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
