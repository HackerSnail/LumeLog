//! User scripts (Roadmap-3). Scripts live in `$APP_CONFIG/LumeLog/scripts` and
//! define `onLine(line, ctx)`; the runner (Task 2) executes them in a locked-down
//! QuickJS runtime over the active document's lines. This module owns the store
//! (list/read/write/delete), name validation and the bounded console buffer.
#![allow(dead_code)] // consumed by the script commands (Task 3)

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;
use tauri::AppHandle;

use crate::dto::IpcError;

pub(crate) const MAX_CONSOLE_MESSAGES: usize = 200;
pub(crate) const MAX_CONSOLE_CHARS: usize = 4096;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptEntry {
    pub name: String,
    pub size_bytes: u64,
    pub modified_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptConsole {
    pub level: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptProgress {
    pub scanned: u64,
    pub written: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptStats {
    pub scanned: u64,
    pub written: u64,
    pub dropped: u64,
    pub cancelled: bool,
    pub truncated_console: bool,
    pub error: Option<String>,
}

/// `[A-Za-z0-9_-]{1,64}.js` — no separators, no traversal, no empty stem.
pub(crate) fn validate_name(name: &str) -> Result<(), IpcError> {
    let stem = name
        .strip_suffix(".js")
        .ok_or_else(|| IpcError::new("invalid_argument", "脚本名必须以 .js 结尾"))?;
    if stem.is_empty() || stem.len() > 64 {
        return Err(IpcError::new(
            "invalid_argument",
            "脚本名主体需 1–64 个字符",
        ));
    }
    if !stem
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(IpcError::new(
            "invalid_argument",
            "脚本名仅允许字母、数字、下划线与连字符",
        ));
    }
    Ok(())
}

/// Bounded console sink: caps each message and the total count; once the cap is
/// hit it emits a single truncation warning.
pub(crate) struct ConsoleBuf {
    count: usize,
    truncated: bool,
}

impl ConsoleBuf {
    pub(crate) fn new() -> Self {
        Self {
            count: 0,
            truncated: false,
        }
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    /// Returns the message to forward, or `None` when it must be dropped.
    pub(crate) fn push(&mut self, level: &str, text: String) -> Option<ScriptConsole> {
        if self.count >= MAX_CONSOLE_MESSAGES {
            if self.truncated {
                return None;
            }
            self.truncated = true;
            return Some(ScriptConsole {
                level: "warn".into(),
                text: "控制台输出已截断".into(),
            });
        }
        let mut text = text;
        if text.chars().count() > MAX_CONSOLE_CHARS {
            text = text.chars().take(MAX_CONSOLE_CHARS).collect();
        }
        self.count += 1;
        Some(ScriptConsole {
            level: level.into(),
            text,
        })
    }
}

pub(crate) fn scripts_dir(app: &AppHandle) -> Result<PathBuf, IpcError> {
    let config = crate::commands::config_path(app)?;
    Ok(config
        .parent()
        .map(|p| p.join("scripts"))
        .unwrap_or_else(|| PathBuf::from("scripts")))
}

pub(crate) fn list_scripts_in(dir: &Path) -> Result<Vec<ScriptEntry>, IpcError> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(IpcError::new("io", format!("read scripts dir: {e}"))),
    };
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if validate_name(name).is_err() {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue, // raced with a delete: skip
        };
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        out.push(ScriptEntry {
            name: name.to_string(),
            size_bytes: meta.len(),
            modified_ms,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub(crate) fn read_script_in(dir: &Path, name: &str) -> Result<String, IpcError> {
    validate_name(name)?;
    std::fs::read_to_string(dir.join(name))
        .map_err(|e| IpcError::new("io", format!("read script: {e}")))
}

/// Atomic write (tmp in the same dir → rename) so a failed save never leaves a
/// half-written script behind.
pub(crate) fn write_script_in(dir: &Path, name: &str, text: &str) -> Result<(), IpcError> {
    validate_name(name)?;
    std::fs::create_dir_all(dir)
        .map_err(|e| IpcError::new("io", format!("create scripts dir: {e}")))?;
    let tmp = dir.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let result = std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, dir.join(name)));
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(IpcError::new("io", format!("write script: {e}")));
    }
    Ok(())
}

pub(crate) fn delete_script_in(dir: &Path, name: &str) -> Result<(), IpcError> {
    validate_name(name)?;
    match std::fs::remove_file(dir.join(name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(IpcError::new("io", format!("delete script: {e}"))),
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lumelog-script-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_name_cases() {
        assert!(validate_name("a.js").is_ok());
        assert!(validate_name("filter-1_x.js").is_ok());
        for bad in [
            "",
            ".js",
            "a",
            "a.txt",
            "a/b.js",
            "../a.js",
            "a b.js",
            "a.js.js",
        ] {
            assert!(validate_name(bad).is_err(), "{bad:?} must be rejected");
        }
        let long = format!("{}.js", "a".repeat(65));
        assert!(validate_name(&long).is_err());
    }

    #[test]
    fn store_roundtrip_and_atomicity() {
        let dir = temp_dir("store");
        assert!(list_scripts_in(&dir).unwrap().is_empty());
        write_script_in(&dir, "a.js", "return line;\n").unwrap();
        assert_eq!(read_script_in(&dir, "a.js").unwrap(), "return line;\n");
        let list = list_scripts_in(&dir).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "a.js");
        assert!(list[0].size_bytes > 0);
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "tmp files must not survive");
        delete_script_in(&dir, "a.js").unwrap();
        delete_script_in(&dir, "a.js").unwrap(); // idempotent
        assert!(list_scripts_in(&dir).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn console_caps_messages_and_count() {
        let mut buf = ConsoleBuf::new();
        let long = "x".repeat(MAX_CONSOLE_CHARS + 10);
        let msg = buf.push("log", long).unwrap();
        assert_eq!(msg.text.chars().count(), MAX_CONSOLE_CHARS);
        for _ in 1..MAX_CONSOLE_MESSAGES {
            assert!(buf.push("log", "x".into()).is_some());
        }
        let truncation = buf.push("log", "overflow".into()).unwrap();
        assert_eq!(truncation.level, "warn");
        assert!(truncation.text.contains("截断"));
        assert!(buf.truncated());
        assert!(buf.push("log", "again".into()).is_none());
    }
}
