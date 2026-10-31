//! Find-in-Files (Plan 44): recursive directory scan with LumeLog's encoding
//! detection, so GBK/GB18030 bench logs match exactly like an open document.
//! Pure/std only — no Tauri types — so every rule is unit-testable.

use crate::encoding;
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};

const DETECT_HEAD: usize = 64 * 1024;
const BATCH: usize = 200;
const LINE_TEXT_CAP: usize = 500;

/// What to match, built from the front end's pattern + mode + case flag.
pub enum FifMatcher {
    Literal { needle: String, case_sensitive: bool },
    Regex(Regex),
}

impl FifMatcher {
    /// `mode`: "literal" | "regex" (anything else is an error).
    pub fn build(pattern: &str, mode: &str, case_sensitive: bool) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err("查找内容为空".to_string());
        }
        match mode {
            "literal" => Ok(Self::Literal {
                needle: pattern.to_string(),
                case_sensitive,
            }),
            "regex" => Regex::new(pattern)
                .map(|r| {
                    if case_sensitive {
                        Self::Regex(r)
                    } else {
                        // Rebuild case-insensitively; the pattern itself may
                        // not carry flags.
                        Self::Regex(
                            Regex::new(&format!("(?i){pattern}"))
                                .unwrap_or(r),
                        )
                    }
                })
                .map_err(|e| format!("正则无效：{e}")),
            other => Err(format!("未知匹配模式 {other}")),
        }
    }

    fn matches(&self, line: &str) -> bool {
        match self {
            Self::Literal { needle, case_sensitive } => {
                if *case_sensitive {
                    line.contains(needle.as_str())
                } else {
                    line.to_lowercase().contains(&needle.to_lowercase())
                }
            }
            Self::Regex(re) => re.is_match(line),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FifHit {
    /// Path relative to the scan root, `/`-separated.
    pub path: String,
    /// 1-based line number.
    pub line: u64,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FifStats {
    pub files: u64,
    pub skipped: u64,
    pub hits: u64,
    pub truncated: bool,
}

const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];

/// Recursively scan `root`. `cancelled` is checked between files; the scan
/// stops as soon as `max_hits` matches are found (`truncated = true`).
pub fn scan_dir(
    root: &Path,
    matcher: &FifMatcher,
    max_hits: usize,
    max_file_bytes: u64,
    on_batch: &mut dyn FnMut(Vec<FifHit>),
    cancelled: &dyn Fn() -> bool,
) -> FifStats {
    let mut stats = FifStats {
        files: 0,
        skipped: 0,
        hits: 0,
        truncated: false,
    };
    let mut batch: Vec<FifHit> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancelled() || stats.truncated {
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => {
                stats.skipped += 1;
                continue;
            }
        };
        for entry in entries.flatten() {
            if cancelled() || stats.truncated {
                break;
            }
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > max_file_bytes {
                stats.skipped += 1;
                continue;
            }
            stats.files += 1;
            scan_file(&path, root, matcher, max_hits, &mut batch, &mut stats, on_batch);
            if stats.hits >= max_hits as u64 {
                stats.truncated = true;
                break;
            }
        }
    }
    if !batch.is_empty() {
        on_batch(std::mem::take(&mut batch));
    }
    stats
}

fn scan_file(
    path: &Path,
    root: &Path,
    matcher: &FifMatcher,
    max_hits: usize,
    batch: &mut Vec<FifHit>,
    stats: &mut FifStats,
    on_batch: &mut dyn FnMut(Vec<FifHit>),
) {
    let Ok(bytes) = fs::read(path) else {
        stats.skipped += 1;
        return;
    };
    let head = &bytes[..bytes.len().min(DETECT_HEAD)];
    let label = encoding::detect(head);
    let text = encoding::decode_all(label, &bytes).text;
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    for (i, line) in text.split('\n').enumerate() {
        if matcher.matches(line) {
            let mut t = line.trim_end_matches('\r').to_string();
            if t.len() > LINE_TEXT_CAP {
                t.truncate(LINE_TEXT_CAP);
            }
            batch.push(FifHit {
                path: rel.clone(),
                line: i as u64 + 1,
                text: t,
            });
            stats.hits += 1;
            if batch.len() >= BATCH {
                on_batch(std::mem::take(batch));
            }
            if stats.hits >= max_hits as u64 {
                stats.truncated = true;
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, bytes: &[u8]) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn literal_and_case_modes_over_mixed_encodings() {
        let dir = std::env::temp_dir().join(format!("fif-mixed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("a.log"), b"start\nCAN tx id=1\nend\n");
        // GBK sample big enough for chardetng to call GBK plausible (a 13-byte
        // file reads as 1252/UTF-8 noise). Encoded via encoding_rs, not hand-
        // assembled bytes.
        let (gbk_bytes, _, _) = encoding_rs::GBK.encode(
            "时间 出错 报警 id=7\n重复 数据 报警 路径\n更多 信息 报警 记录\n数据 报警 时间 记录\n",
        );
        write(&dir.join("gbk.log"), &gbk_bytes);
        write(&dir.join(".hide").join("x.log"), b"CAN tx hidden\n");
        write(&dir.join(".gitless.log"), b"CAN tx in dotfile\n");

        // Case-sensitive literal misses; insensitive hits.
        let cs = FifMatcher::build("can tx", "literal", true).unwrap();
        let stats = scan_dir(&dir, &cs, 10_000, 1 << 20, &mut |_| {}, &|| false);
        assert_eq!(stats.hits, 0);
        let ci = FifMatcher::build("can tx", "literal", false).unwrap();
        let mut hits: Vec<FifHit> = Vec::new();
        let stats = scan_dir(&dir, &ci, 10_000, 1 << 20, &mut |b| hits.extend(b), &|| false);
        // a.log + .gitless.log (dot FILES are scanned; dot DIRS are skipped)
        assert_eq!(stats.hits, 2);
        assert!(hits.iter().all(|h| !h.path.contains('\\')));

        // GBK content matches after decode (literal "报警").
        let gbk = FifMatcher::build("报警", "literal", true).unwrap();
        let mut ghits: Vec<FifHit> = Vec::new();
        scan_dir(&dir, &gbk, 10_000, 1 << 20, &mut |b| ghits.extend(b), &|| false);
        assert_eq!(ghits.len(), 4, "all four GBK lines contain 报警");
        assert!(ghits[0].path.starts_with("gbk.log"));
        assert!(ghits[0].text.contains("id=7"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn regex_mode_and_truncation() {
        let dir = std::env::temp_dir().join(format!("fif-re-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut big = String::new();
        for i in 0..300 {
            big.push_str(&format!("row {i} val={i}\n"));
        }
        write(&dir.join("big.log"), big.as_bytes());
        let re = FifMatcher::build("val=\\d+", "regex", true).unwrap();
        let stats = scan_dir(&dir, &re, 10, 1 << 20, &mut |_| {}, &|| false);
        assert_eq!(stats.hits, 10);
        assert!(stats.truncated);

        assert!(FifMatcher::build("([", "regex", true).is_err());
        assert!(FifMatcher::build("", "literal", true).is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_oversize_files_dot_dirs_and_respects_cancel() {
        let dir = std::env::temp_dir().join(format!("fif-skip-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        write(&dir.join("small.log"), b"needle\n");
        write(&dir.join("huge.log"), &[b'x'; 100]);
        write(&dir.join("target").join("in.log"), b"needle\n");
        let m = FifMatcher::build("needle", "literal", true).unwrap();
        let stats = scan_dir(&dir, &m, 10_000, 50, &mut |_| {}, &|| false);
        assert_eq!(stats.hits, 1); // only small.log; huge skipped, target dir skipped
        assert_eq!(stats.skipped, 1);

        let stats = scan_dir(&dir, &m, 10_000, 50, &mut |_| {}, &|| true);
        assert_eq!(stats.hits, 0, "cancelled before any file");
        fs::remove_dir_all(&dir).ok();
    }
}
