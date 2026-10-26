//! MF4 (ASAM MDF4) measurement sessions — Roadmap-4 increment 1. This module is
//! the only place that touches `falcon_mdf`; everything above it sees our own
//! DTOs, so the library can be swapped without touching the UI contract.
//!
//! An MF4 file is not a text document: it opens as a *session* (id + handle +
//! cached channel list) and renders in its own dock. Sample data is read lazily
//! and windowed, so no call here materialises a whole signal.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use falcon_mdf::{Channel, Mf4File, SignalValues};
use serde::Serialize;

use crate::dto::IpcError;

/// How many measurement files may stay open at once (mmap handles + index).
pub(crate) const MAX_SESSIONS: usize = 4;
/// Column cap for one table page / export run.
pub(crate) const MAX_COLUMNS: usize = 32;
/// Hard cap on rows returned by one `records` call.
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
    pub file: Arc<Mutex<Mf4File>>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Mf4CellJson {
    Number(f64),
    Text(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4ColumnJson {
    pub index: u32,
    pub name: String,
    pub unit: String,
    pub values: Vec<Mf4CellJson>,
    /// The channel exists but could not be read as cells (odd type / failure).
    pub degraded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4RecordsJson {
    /// Milliseconds since measurement start, `None` when the file has no time axis.
    pub time_ms: Vec<Option<f64>>,
    pub columns: Vec<Mf4ColumnJson>,
    pub total_rows: u64,
    pub has_time_axis: bool,
}

/// Sample values of one channel, collected window-first.
///
/// Streams the signal in `signal_chunks` windows and keeps only the samples named
/// by `rows` (absolute, ascending), so memory stays bounded by the page instead of
/// the signal length. Text/bytes channels land in `Text`, everything numeric in
/// `Number`; a sample the file marks invalid (or a chunk that fails) becomes
/// `Null`, and a channel that cannot be read at all sets `degraded`.
fn cells_for_rows(file: &Mf4File, ch: &Channel, rows: &[usize]) -> (Vec<Mf4CellJson>, bool) {
    let wanted: usize = rows.len();
    let mut out = vec![Mf4CellJson::Null; wanted];
    let mut cursor = 0usize; // index into `rows`
    let mut base: usize = 0; // absolute sample index of the current chunk
    let chunks = match file.signal_chunks(ch) {
        Ok(c) => c,
        Err(_) => return (out, true),
    };
    // MDF stores booleans as integers; report them as `Bool` so the UI can render
    // a tick instead of 0/1.
    let as_bool = kind_of(ch) == Mf4Kind::Bool;
    for chunk in chunks {
        let chunk = match chunk {
            Ok(c) => c,
            Err(_) => return (out, true),
        };
        let len = chunk.len();
        let vals = match chunk.values() {
            Ok(v) => v,
            Err(_) => return (out, true),
        };
        while cursor < wanted && rows[cursor] < base + len {
            let local = rows[cursor] - base;
            out[cursor] = {
                let cell = cell_of(&vals, local, chunk.is_valid(local));
                match (as_bool, cell) {
                    (true, Mf4CellJson::Number(v)) => Mf4CellJson::Bool(v != 0.0),
                    (_, other) => other,
                }
            };
            cursor += 1;
        }
        base += len;
        if cursor >= wanted {
            break;
        }
    }
    (out, false)
}

fn cell_of(values: &SignalValues, local: usize, valid: bool) -> Mf4CellJson {
    if !valid {
        return Mf4CellJson::Null;
    }
    macro_rules! num {
        ($v:expr, $i:expr) => {
            $v.get($i)
                .map(|x| Mf4CellJson::Number(*x as f64))
                .unwrap_or(Mf4CellJson::Null)
        };
    }
    match values {
        SignalValues::U8(v) => num!(v, local),
        SignalValues::U16(v) => num!(v, local),
        SignalValues::U32(v) => num!(v, local),
        SignalValues::U64(v) => num!(v, local),
        SignalValues::I8(v) => num!(v, local),
        SignalValues::I16(v) => num!(v, local),
        SignalValues::I32(v) => num!(v, local),
        SignalValues::I64(v) => num!(v, local),
        SignalValues::F32(v) => num!(v, local),
        SignalValues::F64(v) => num!(v, local),
        SignalValues::Str(v) => v
            .get(local)
            .cloned()
            .map(Mf4CellJson::Text)
            .unwrap_or(Mf4CellJson::Null),
        // Anything else (bool, byte arrays, ratios, …) has no cell kind yet.
        _ => Mf4CellJson::Null,
    }
}

/// Absolute sample rows inside `[from,to]` seconds, ordered by time and paged.
/// Also returns the millisecond axis and the total in-window sample count (so the
/// UI can page accurately); `axis` is empty when the file has no time channel.
fn select_rows(
    file: &Mf4File,
    master: Option<&Channel>,
    from: Option<f64>,
    to: Option<f64>,
    desc: bool,
    limit: usize,
    offset: usize,
) -> Result<(Vec<usize>, Vec<f64>, u64, bool), IpcError> {
    // (sample index, time in ms) for every in-window sample, file order.
    let mut pairs: Vec<(usize, f64)> = Vec::new();
    let has_time_axis = master.is_some();
    if let Some(m) = master {
        let mut base: usize = 0;
        for chunk in file.signal_chunks(m).map_err(classify)? {
            let chunk = chunk.map_err(classify)?;
            let len = chunk.len();
            for (i, t) in chunk.values_f64().map_err(classify)?.iter().enumerate() {
                let in_window = from.is_none_or(|f| *t >= f) && to.is_none_or(|v| *t <= v);
                if in_window {
                    pairs.push((base + i, t * 1000.0));
                }
            }
            base += len;
        }
    } else {
        // No time axis at all: pages are plain sample indices of the first group.
        let Some(first) = file.channels().next() else {
            return Ok((Vec::new(), Vec::new(), 0, false));
        };
        let len = file.signal(first).map_err(classify)?.len();
        pairs.extend((0..len).map(|i| (i, 0.0)));
    }
    let total_rows = pairs.len() as u64;
    if desc {
        pairs.reverse();
    }
    let page: Vec<(usize, f64)> = pairs.into_iter().skip(offset).take(limit).collect();
    Ok((
        page.iter().map(|p| p.0).collect(),
        page.iter().map(|p| p.1).collect(),
        total_rows,
        has_time_axis,
    ))
}

/// One page of the record table for the selected channels.
#[allow(clippy::too_many_arguments)] // window + paging + selection in one call
pub(crate) fn records(
    sessions: &Mf4Sessions,
    id: u64,
    channels: &[u32],
    from_sec: Option<f64>,
    to_sec: Option<f64>,
    sort: &str,
    limit: u32,
    offset: u32,
) -> Result<Mf4RecordsJson, IpcError> {
    if channels.is_empty() {
        return Err(IpcError::new("invalid_argument", "请先勾选至少一个通道"));
    }
    if channels.len() > MAX_COLUMNS {
        return Err(IpcError::new(
            "invalid_argument",
            format!("一次最多选择 {MAX_COLUMNS} 个通道"),
        ));
    }
    if limit == 0 || limit > MAX_LIMIT {
        return Err(IpcError::new(
            "invalid_argument",
            format!("每页行数需在 1–{MAX_LIMIT}"),
        ));
    }
    if let (Some(f), Some(t)) = (from_sec, to_sec) {
        if f > t {
            return Err(IpcError::new(
                "invalid_argument",
                "时间窗起点不能晚于止点",
            ));
        }
    }
    let desc = match sort {
        "timeDesc" => true,
        "timeAsc" | "" => false,
        other => {
            return Err(IpcError::new(
                "invalid_argument",
                format!("未知排序方式 {other}"),
            ))
        }
    };
    let session = sessions.get(id).ok_or_else(|| unknown_session(id))?;
    let guard = session.lock().unwrap_or_else(|p| p.into_inner());
    let mut picked: Vec<(Mf4ChannelJson, Channel)> = Vec::with_capacity(channels.len());
    for want in channels {
        let found = guard
            .channels
            .iter()
            .find(|(m, _)| m.index == *want)
            .ok_or_else(|| IpcError::new("invalid_argument", format!("未知通道 {want}")))?;
        picked.push(found.clone());
    }
    let file = guard.file.lock().unwrap_or_else(|p| p.into_inner());
    let master = guard.channels.iter().find(|(_, c)| c.is_master()).map(|(_, c)| c);
    let (rows, time_ms, total_rows, has_time_axis) =
        select_rows(&file, master, from_sec, to_sec, desc, limit as usize, offset as usize)?;
    let mut columns = Vec::with_capacity(picked.len());
    for (meta, ch) in &picked {
        let (values, degraded) = cells_for_rows(&file, ch, &rows);
        columns.push(Mf4ColumnJson {
            index: meta.index,
            name: meta.name.clone(),
            unit: meta.unit.clone(),
            values,
            degraded,
        });
    }
    Ok(Mf4RecordsJson {
        time_ms: if has_time_axis {
            time_ms.into_iter().map(Some).collect()
        } else {
            vec![None; rows.len()]
        },
        columns,
        total_rows,
        has_time_axis,
    })
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4GroupJson {
    pub index: u32,
    pub channels: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4SummaryJson {
    pub version: String,
    pub path: String,
    pub comment: String,
    pub channel_count: u32,
    pub data_groups: u32,
    pub events: u32,
    pub attachments: u32,
    pub groups: Vec<Mf4GroupJson>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4ExportProgress {
    pub written: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mf4ExportStats {
    pub written: u64,
    pub cancelled: bool,
    pub errored: bool,
}

pub(crate) fn summary(sessions: &Mf4Sessions, id: u64) -> Result<Mf4SummaryJson, IpcError> {
    let session = sessions.get(id).ok_or_else(|| unknown_session(id))?;
    let guard = session.lock().unwrap_or_else(|p| p.into_inner());
    let file = guard.file.lock().unwrap_or_else(|p| p.into_inner());
    let mut groups: HashMap<u32, u32> = HashMap::new();
    for (meta, _) in &guard.channels {
        *groups.entry(meta.group).or_insert(0) += 1;
    }
    let mut groups: Vec<Mf4GroupJson> = groups
        .into_iter()
        .map(|(index, channels)| Mf4GroupJson { index, channels })
        .collect();
    groups.sort_by_key(|g| g.index);
    Ok(Mf4SummaryJson {
        version: format!("{}", file.version()),
        path: guard.path.to_string_lossy().into_owned(),
        comment: file.comment().to_string(),
        channel_count: guard.channels.len() as u32,
        data_groups: file.data_group_count() as u32,
        events: file.events().len() as u32,
        attachments: file.attachments().len() as u32,
        groups,
    })
}

/// CSV cell text: numbers trimmed, text quoted, invalid/missing empty.
fn csv_cell(c: &Mf4CellJson) -> String {
    match c {
        Mf4CellJson::Number(v) => {
            if v.fract() == 0.0 && v.abs() < 1e15 {
                format!("{v:.0}")
            } else {
                format!("{v}")
            }
        }
        Mf4CellJson::Text(s) => format!("\"{}\"", s.replace('"', "\"\"")),
        Mf4CellJson::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Mf4CellJson::Null => String::new(),
    }
}

fn csv_header(columns: &[(Mf4ChannelJson, Channel)]) -> String {
    let mut out = String::from("timeMs");
    for (meta, _) in columns {
        out.push(',');
        if meta.unit.is_empty() {
            out.push_str(&format!("\"{}\"", meta.name.replace('"', "\"\"")));
        } else {
            out.push_str(&format!(
                "\"{} ({})\"",
                meta.name.replace('"', "\"\""),
                meta.unit
            ));
        }
    }
    out.push('\n');
    out
}

/// Stream the selected channels in the window to `target` as CSV. Batches of~500
/// rows are rendered through the same cell mapping as the table, so export and UI
/// cannot drift. Cancelling or failing still leaves a readable prefix (tmp→rename).
#[allow(clippy::too_many_arguments)]
pub(crate) fn export_csv(
    sessions: &Mf4Sessions,
    id: u64,
    channels: &[u32],
    from_sec: Option<f64>,
    to_sec: Option<f64>,
    target: &Path,
    on_progress: &mut dyn FnMut(u64),
    cancel: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Mf4ExportStats {
    let mut stats = Mf4ExportStats {
        written: 0,
        cancelled: false,
        errored: false,
    };
    if channels.is_empty() || channels.len() > MAX_COLUMNS {
        stats.errored = true;
        return stats;
    }
    let session = match sessions.get(id) {
        Some(s) => s,
        None => {
            stats.errored = true;
            return stats;
        }
    };
    let guard = session.lock().unwrap_or_else(|p| p.into_inner());
    let mut picked: Vec<(Mf4ChannelJson, Channel)> = Vec::with_capacity(channels.len());
    for want in channels {
        match guard.channels.iter().find(|(m, _)| m.index == *want) {
            Some(found) => picked.push(found.clone()),
            None => {
                stats.errored = true;
                return stats;
            }
        }
    }
    let tmp = target.with_file_name(format!(
        ".{}.tmp-{}-{}",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mf4-out"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let file = match File::create(&tmp) {
        Ok(f) => f,
        Err(_) => {
            stats.errored = true;
            return stats;
        }
    };
    let mut writer = BufWriter::new(file);
    if writer.write_all(csv_header(&picked).as_bytes()).is_err() {
        let _ = std::fs::remove_file(&tmp);
        stats.errored = true;
        return stats;
    }
    let inner = guard.file.lock().unwrap_or_else(|p| p.into_inner());
    let master = guard.channels.iter().find(|(_, c)| c.is_master()).map(|(_, c)| c);

    // Collector: batches of (row, ms) that render and flush together.
    let mut batch: Vec<(usize, f64)> = Vec::with_capacity(500);
    let mut written: u64 = 0;
    let mut last_report: u64 = 0;
    let mut io_failed = false;
    let mut write_batch = |batch: &mut Vec<(usize, f64)>| -> bool {
        if batch.is_empty() {
            return true;
        }
        let rows: Vec<usize> = batch.iter().map(|p| p.0).collect();
        let ms: Vec<f64> = batch.iter().map(|p| p.1).collect();
        let cells: Vec<Vec<Mf4CellJson>> = picked
            .iter()
            .map(|(_, ch)| cells_for_rows(&inner, ch, &rows).0)
            .collect();
        for (k, &row) in rows.iter().enumerate() {
            let mut line = if master.is_some() {
                format!("{:.3}", ms[k])
            } else {
                row.to_string()
            };
            for col in &cells {
                line.push(',');
                line.push_str(&csv_cell(&col[k]));
            }
            line.push('\n');
            if writer.write_all(line.as_bytes()).is_err() {
                return false;
            }
            written += 1;
        }
        if written - last_report >= 50_000 {
            last_report = written;
            on_progress(written);
        }
        batch.clear();
        true
    };

    if let Some(m) = master {
        let mut base: usize = 0;
        let chunks = inner.signal_chunks(m);
        if let Ok(chunks) = chunks {
        'outer: for chunk in chunks {
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => {
                    io_failed = true;
                    break;
                }
            };
            let len = chunk.len();
            let vals = match chunk.values_f64() {
                Ok(v) => v,
                Err(_) => {
                    io_failed = true;
                    break;
                }
            };
            for (i, t) in vals.iter().enumerate() {
                if cancel() {
                    stats.cancelled = true;
                    break 'outer;
                }
                let in_window = from_sec.is_none_or(|f| *t >= f) && to_sec.is_none_or(|v| *t <= v);
                if in_window {
                    batch.push((base + i, t * 1000.0));
                    if batch.len() >= 500 && !write_batch(&mut batch) {
                        io_failed = true;
                        break 'outer;
                    }
                }
            }
            base += len;
        }
        } else {
            io_failed = true;
        }
    } else if let Some((_, first)) = picked.first() {
        let len = inner.signal(first).map(|s| s.len()).unwrap_or(0);
        for row in 0..len {
            if cancel() {
                stats.cancelled = true;
                break;
            }
            batch.push((row, 0.0));
            if batch.len() >= 500 && !write_batch(&mut batch) {
                io_failed = true;
                break;
            }
        }
    }
    if !io_failed && !write_batch(&mut batch) {
        io_failed = true;
    }
    drop(write_batch);
    stats.written = written;
    if writer.flush().is_err() {
        io_failed = true;
    }
    if io_failed {
        let _ = std::fs::remove_file(&tmp);
        stats.errored = true;
        return stats;
    }
    if std::fs::rename(&tmp, target).is_err() {
        let _ = std::fs::remove_file(&tmp);
        stats.errored = true;
        return stats;
    }
    on_progress(stats.written);
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use falcon_mdf::Mf4Writer;
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

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
            // Sample 1 is flagged invalid so `records` can prove null mapping.
            g.add_channel_with_validity(
                "EngineTemp",
                "degC",
                &[1.5, 2.0, 2.5, 3.0],
                Some(&[true, false, true, true]),
            )
            .unwrap();
            g.add_channel_vlsd_str("Note", "", &["a\"b", "ok", "", "z"])
                .unwrap();
        }
        w.write_to_file(&path).unwrap();
        (dir, path)
    }

    #[test]
    fn export_csv_matches_records_and_escapes() {
        let (dir, path) = fixture("export");
        let sessions = Mf4Sessions::default();
        let info = open_session(&sessions, &path).unwrap();
        let speed = channel_id(&sessions, info.id, "Speed");
        let note = channel_id(&sessions, info.id, "Note");
        let target = dir.join("out.csv");
        let mut seen = 0u64;
        let stats = export_csv(
            &sessions,
            info.id,
            &[speed, note],
            None,
            None,
            &target,
            &mut |n| seen = n,
            Arc::new(|| false),
        );
        assert!(!stats.errored && !stats.cancelled);
        assert_eq!(stats.written, 4);
        assert_eq!(seen, 4);
        let text = std::fs::read_to_string(&target).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "header + 4 rows");
        assert!(lines[0].starts_with("timeMs"), "{}", lines[0]);
        assert!(lines[0].contains("Speed (km/h)"));
        assert!(lines[1].starts_with("0.000,10,"));
        assert!(lines[1].contains("\"a\"\"b\""), "quotes doubled: {}", lines[1]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn export_rejects_empty_selection_and_honours_cancel() {
        let (dir, path) = fixture("export-cancel");
        let sessions = Mf4Sessions::default();
        let info = open_session(&sessions, &path).unwrap();
        let speed = channel_id(&sessions, info.id, "Speed");
        let mut none = |_n: u64| {};
        let empty = export_csv(
            &sessions,
            info.id,
            &[],
            None,
            None,
            &dir.join("empty.csv"),
            &mut none,
            Arc::new(|| false),
        );
        assert!(empty.errored, "no columns is a caller error");

        let flag = Arc::new(AtomicBool::new(true));
        let f2 = flag.clone();
        let cancel_target = dir.join("cancel.csv");
        let cancelled = export_csv(
            &sessions,
            info.id,
            &[speed],
            None,
            None,
            &cancel_target,
            &mut none,
            Arc::new(move || f2.load(AtomicOrdering::Relaxed)),
        );
        assert!(cancelled.cancelled);
        assert_eq!(cancelled.written, 0);
        assert!(cancel_target.exists(), "cancelled run keeps the header prefix");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn summary_lists_groups_and_counts() {
        let (dir, path) = fixture("summary");
        let sessions = Mf4Sessions::default();
        let info = open_session(&sessions, &path).unwrap();
        let s = summary(&sessions, info.id).unwrap();
        assert_eq!(s.channel_count, info.channel_count);
        assert_eq!(s.data_groups, 1);
        assert_eq!(s.groups.iter().map(|g| g.channels).sum::<u32>(), s.channel_count);
        assert!(!s.version.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn records_window_sort_and_paginate() {
        let (dir, path) = fixture("records");
        let sessions = Mf4Sessions::default();
        let info = open_session(&sessions, &path).unwrap();
        let speed = channel_id(&sessions, info.id, "Speed");
        let temp = channel_id(&sessions, info.id, "EngineTemp");

        let all = records(&sessions, info.id, &[speed, temp], None, None, "timeAsc", 10, 0).unwrap();
        assert_eq!(all.total_rows, 4);
        assert!(all.has_time_axis);
        assert_eq!(all.time_ms, vec![Some(0.0), Some(1000.0), Some(2000.0), Some(3000.0)]);
        assert_eq!(all.columns.len(), 2);
        assert_eq!(cell_num(&all.columns[0].values[3]), 40.0);

        let clipped =
            records(&sessions, info.id, &[speed], Some(1.0), Some(2.0), "timeAsc", 10, 0).unwrap();
        assert_eq!(clipped.total_rows, 2);
        assert_eq!(clipped.time_ms, vec![Some(1000.0), Some(2000.0)]);

        let desc = records(&sessions, info.id, &[speed], None, None, "timeDesc", 2, 0).unwrap();
        assert_eq!(desc.time_ms, vec![Some(3000.0), Some(2000.0)]);
        assert_eq!(cell_num(&desc.columns[0].values[0]), 40.0);

        let p2 = records(&sessions, info.id, &[speed], None, None, "timeAsc", 2, 2).unwrap();
        assert_eq!(p2.total_rows, 4, "total is the whole window, not the page");
        assert_eq!(cell_num(&p2.columns[0].values[0]), 30.0);

        // Column order follows the requested selection, not the file order.
        let flipped =
            records(&sessions, info.id, &[temp, speed], None, None, "timeAsc", 1, 0).unwrap();
        assert_eq!(flipped.columns[0].name, "EngineTemp");
        assert_eq!(flipped.columns[1].name, "Speed");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn records_marks_invalid_samples_and_rejects_bad_requests() {
        let (dir, path) = fixture("invalid");
        let sessions = Mf4Sessions::default();
        let info = open_session(&sessions, &path).unwrap();
        let temp = channel_id(&sessions, info.id, "EngineTemp");
        let rows = records(&sessions, info.id, &[temp], None, None, "timeAsc", 10, 0).unwrap();
        assert!(matches!(rows.columns[0].values[1], Mf4CellJson::Null), "marked invalid");
        assert!(matches!(rows.columns[0].values[0], Mf4CellJson::Number(_)));

        assert_eq!(records(&sessions, info.id, &[], None, None, "timeAsc", 10, 0).unwrap_err().code, "invalid_argument");
        assert_eq!(
            records(&sessions, info.id, &[temp], None, None, "timeAsc", MAX_LIMIT + 1, 0).unwrap_err().code,
            "invalid_argument"
        );
        assert_eq!(
            records(&sessions, info.id, &[temp], Some(5.0), Some(1.0), "timeAsc", 10, 0).unwrap_err().code,
            "invalid_argument"
        );
        assert_eq!(
            records(&sessions, info.id, &[temp], None, None, "col:3Asc", 10, 0).unwrap_err().code,
            "invalid_argument"
        );
        assert_eq!(
            records(&sessions, info.id, &[9999], None, None, "timeAsc", 10, 0).unwrap_err().code,
            "invalid_argument"
        );
        let too_many: Vec<u32> = (0..=(MAX_COLUMNS as u32)).collect();
        assert_eq!(
            records(&sessions, info.id, &too_many, None, None, "timeAsc", 10, 0).unwrap_err().code,
            "invalid_argument"
        );
        std::fs::remove_dir_all(&dir).ok();
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

    fn channel_id(sessions: &Mf4Sessions, id: u64, name: &str) -> u32 {
        list_channels(sessions, id, Some(name), 10, 0)
            .unwrap()
            .items
            .first()
            .unwrap_or_else(|| panic!("channel {name} missing"))
            .index
    }

    fn cell_num(c: &Mf4CellJson) -> f64 {
        match c {
            Mf4CellJson::Number(v) => *v,
            other => panic!("expected number cell, got {other:?}"),
        }
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
