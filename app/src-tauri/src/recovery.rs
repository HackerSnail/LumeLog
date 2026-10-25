//! Crash-recovery shadow store (spec §4.4/§10, M3 子项目 B1). Each dirty
//! edit-mode doc gets one shadow file under `<app_config>/LumeLog/recovery`:
//! a single JSON metadata line + `\n` + the raw UTF-8 text (so a large document
//! needs no JSON escaping). `doc_key = FNV-1a64(path)` keeps one shadow per file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// On-disk shadow header. `saved_at_ms` is Unix epoch milliseconds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryMeta {
    pub path: String,
    pub encoding: String,
    pub eol: String,
    pub saved_at_ms: u64,
}

/// One listed shadow (`recovery_list` row).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryEntry {
    pub path: String,
    pub encoding: String,
    pub eol: String,
    pub saved_at_ms: u64,
    pub size_bytes: u64,
}

/// A loaded shadow (`recovery_load` payload).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryContent {
    pub path: String,
    pub encoding: String,
    pub eol: String,
    pub text: String,
}

/// Standard FNV-1a 64-bit hash.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Stable 16-hex shadow key for an original file path.
pub fn doc_key(path: &str) -> String {
    format!("{:016x}", fnv1a64(path.as_bytes()))
}

/// `<dir>/<doc_key>.lumelog`.
pub fn shadow_path(dir: &Path, path: &str) -> PathBuf {
    dir.join(format!("{}.lumelog", doc_key(path)))
}

/// Write (or overwrite) the shadow for `meta.path`: JSON header + `\n` + text,
/// via a sibling `.lumelogtmp` then rename (a shadow is not authoritative data).
pub fn write_shadow(dir: &Path, meta: &RecoveryMeta, text: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let target = shadow_path(dir, &meta.path);
    let tmp = target.with_extension("lumelogtmp");
    let header = serde_json::to_string(meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut buf = Vec::with_capacity(header.len() + 1 + text.len());
    buf.extend_from_slice(header.as_bytes());
    buf.push(b'\n');
    buf.extend_from_slice(text.as_bytes());
    fs::write(&tmp, &buf)?;
    fs::rename(&tmp, &target)?;
    Ok(())
}

/// Read the shadow for `path` (meta + exact text). A missing/corrupt shadow is `io`.
pub fn read_shadow(dir: &Path, path: &str) -> io::Result<(RecoveryMeta, String)> {
    let raw = fs::read(shadow_path(dir, path))?;
    split_shadow(&raw)
}

/// Split a shadow blob at its FIRST `\n` (the JSON header never contains a raw
/// newline, serde escapes it) into meta + text.
fn split_shadow(raw: &[u8]) -> io::Result<(RecoveryMeta, String)> {
    let idx = raw
        .iter()
        .position(|b| *b == b'\n')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "shadow has no metadata line"))?;
    let meta: RecoveryMeta = serde_json::from_slice(&raw[..idx])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let text = String::from_utf8(raw[idx + 1..].to_vec())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((meta, text))
}

/// All shadows under `dir` (newest first). A missing dir -> empty; a corrupt
/// shadow is skipped rather than aborting the whole list.
pub fn list_shadows(dir: &Path) -> Vec<RecoveryEntry> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let p = ent.path();
        if p.extension().and_then(|e| e.to_str()) != Some("lumelog") {
            continue;
        }
        let raw = match fs::read(&p) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let (meta, _text) = match split_shadow(&raw) {
            Ok(v) => v,
            Err(_) => continue, // corrupt shadow -> skip
        };
        out.push(RecoveryEntry {
            path: meta.path,
            encoding: meta.encoding,
            eol: meta.eol,
            saved_at_ms: meta.saved_at_ms,
            size_bytes: raw.len() as u64,
        });
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.saved_at_ms));
    out
}

/// Delete the shadow for `path`; a missing file is success (idempotent),
/// but any other I/O failure is reported so callers cannot silently orphan it.
pub fn discard_shadow(dir: &Path, path: &str) -> io::Result<()> {
    match fs::remove_file(shadow_path(dir, path)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

// ---- minimal config switch (config.toml `recovery_enabled`) ----------------

/// Parse `recovery_enabled` from a `config.toml` body. Missing, non-bool, or an
/// unparseable file all mean "enabled" (the default is on).
pub fn read_recovery_enabled(text: &str) -> bool {
    toml::from_str::<toml::Table>(text)
        .ok()
        .and_then(|t| t.get("recovery_enabled").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}

/// Rewrite a `config.toml` body with `recovery_enabled = enabled`, preserving
/// other keys. Unparseable input is replaced by a minimal one-key file.
pub fn set_recovery_enabled(text: &str, enabled: bool) -> String {
    let mut table: toml::Table = toml::from_str(text).unwrap_or_default();
    table.insert(
        "recovery_enabled".to_string(),
        toml::Value::Boolean(enabled),
    );
    toml::to_string_pretty(&table)
        .unwrap_or_else(|_| format!("recovery_enabled = {enabled}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn meta(path: &str, saved_at_ms: u64) -> RecoveryMeta {
        RecoveryMeta {
            path: path.to_string(),
            encoding: "GBK".to_string(),
            eol: "CRLF".to_string(),
            saved_at_ms,
        }
    }

    #[test]
    fn fnv_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn doc_key_stable_and_distinct() {
        assert_eq!(doc_key("C:/logs/a.log"), doc_key("C:/logs/a.log"));
        assert_ne!(doc_key("C:/logs/a.log"), doc_key("C:/logs/b.log"));
        assert_eq!(doc_key("C:/logs/a.log").len(), 16);
    }

    #[test]
    fn roundtrip_meta_and_text() {
        let d = tempdir().unwrap();
        let text = "第一行\nsecond line 🚀\r\n  缩进 emoji 😀\n";
        let m = meta("C:/logs/a.log", 1_700_000_000_000);
        write_shadow(d.path(), &m, text).unwrap();
        let (back, t) = read_shadow(d.path(), "C:/logs/a.log").unwrap();
        assert_eq!(back, m);
        assert_eq!(t, text);

        // The first line is exactly the JSON header (serde escapes any newline).
        let raw = fs::read(shadow_path(d.path(), "C:/logs/a.log")).unwrap();
        let nl = raw.iter().position(|b| *b == b'\n').unwrap();
        let head: RecoveryMeta = serde_json::from_slice(&raw[..nl]).unwrap();
        assert_eq!(head.path, "C:/logs/a.log");
        assert_eq!(head.encoding, "GBK");
    }

    #[test]
    fn write_overwrites_same_key() {
        let d = tempdir().unwrap();
        write_shadow(d.path(), &meta("C:/a.log", 1), "one").unwrap();
        write_shadow(d.path(), &meta("C:/a.log", 2), "two").unwrap();
        let (_m, t) = read_shadow(d.path(), "C:/a.log").unwrap();
        assert_eq!(t, "two");
        assert_eq!(list_shadows(d.path()).len(), 1);
    }

    #[test]
    fn list_sorted_and_skips_bad() {
        let d = tempdir().unwrap();
        write_shadow(d.path(), &meta("C:/old.log", 100), "old").unwrap();
        write_shadow(d.path(), &meta("C:/new.log", 200), "new").unwrap();
        fs::write(d.path().join("corrupt.lumelog"), b"no-newline-here").unwrap();
        let list = list_shadows(d.path());
        assert_eq!(list.len(), 2, "corrupt shadow skipped");
        assert_eq!(list[0].saved_at_ms, 200, "newest first");
        assert_eq!(list[1].saved_at_ms, 100);
        assert!(list.iter().all(|e| e.size_bytes > 0));
    }

    #[test]
    fn list_missing_dir_empty() {
        let d = tempdir().unwrap();
        assert!(list_shadows(&d.path().join("nope")).is_empty());
    }

    #[test]
    fn discard_idempotent() {
        let d = tempdir().unwrap();
        write_shadow(d.path(), &meta("C:/a.log", 1), "x").unwrap();
        discard_shadow(d.path(), "C:/a.log").unwrap();
        assert!(read_shadow(d.path(), "C:/a.log").is_err());
        discard_shadow(d.path(), "C:/a.log").unwrap(); // idempotent: NotFound is success
        assert!(list_shadows(d.path()).is_empty());
    }

    #[test]
    fn cfg_default_true_when_missing_or_bad() {
        assert!(read_recovery_enabled(""));
        assert!(read_recovery_enabled("bad [ toml"));
        assert!(read_recovery_enabled("theme = \"dark\"\n"));
        assert!(!read_recovery_enabled("recovery_enabled = false\n"));
        assert!(read_recovery_enabled("recovery_enabled = true\n"));
    }

    #[test]
    fn cfg_set_preserves_other_keys() {
        let out = set_recovery_enabled("theme = \"dark\"\n", false);
        assert!(out.contains("theme = \"dark\""), "{out}");
        assert!(!read_recovery_enabled(&out));
        let back = set_recovery_enabled(&out, true);
        assert!(read_recovery_enabled(&back));
        assert!(back.contains("theme = \"dark\""));
    }

    #[test]
    fn cfg_set_on_bad_text_rewrites() {
        let out = set_recovery_enabled("bad [ toml", false);
        assert!(!read_recovery_enabled(&out));
    }

    #[test]
    fn entry_and_content_camel_case() {
        let e = RecoveryEntry {
            path: "C:/a.log".into(),
            encoding: "UTF-8".into(),
            eol: "LF".into(),
            saved_at_ms: 7,
            size_bytes: 9,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["savedAtMs"], 7);
        assert_eq!(v["sizeBytes"], 9);
        assert_eq!(v["path"], "C:/a.log");

        let c = RecoveryContent {
            path: "C:/a.log".into(),
            encoding: "GBK".into(),
            eol: "CRLF".into(),
            text: "x".into(),
        };
        let vc = serde_json::to_value(&c).unwrap();
        assert_eq!(vc["text"], "x");
        assert_eq!(vc["eol"], "CRLF");
    }
}
