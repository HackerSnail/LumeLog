//! Inspector Git section (Plan 34): repo probe + `git init` via the system git
//! CLI. Read-only besides init — no staging/commit surface. All subprocess
//! calls pass an argument array (no shell), cwd is the document's directory.

use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Serialize)]
pub struct GitInfo {
    pub repo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit: Option<LastCommit>,
    /// Some(msg) when the git binary itself is unusable (not on PATH …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LastCommit {
    pub hash: String,
    pub subject: String,
    /// Unix seconds (front end formats).
    pub time: i64,
}

fn run_git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git 不可用：{e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

/// Map the first `XY path` line of `git status --porcelain` to a zh label.
/// None = clean (no output line for the file).
pub fn porcelain_file_status(out: &str) -> Option<String> {
    let line = out.lines().find(|l| !l.trim().is_empty())?;
    let bytes = line.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    let (x, y) = (bytes[0], bytes[1]);
    let label = match (x, y) {
        (b'?', _) => "未跟踪".to_string(),
        (b'!', _) => "已忽略".to_string(),
        (b'M', _) => "已暂存（修改）".to_string(),
        (b'A', _) => "已暂存（新增）".to_string(),
        (b'D', _) => "已暂存（删除）".to_string(),
        (b'R', _) => "已暂存（重命名）".to_string(),
        (b'C', _) => "已暂存（复制）".to_string(),
        (_, b'M') => "已修改（未暂存）".to_string(),
        (_, b'D') => "已删除（未暂存）".to_string(),
        _ => format!("状态 {}{}", x as char, y as char),
    };
    Some(label)
}

/// Probe the repo containing `dir`, reporting `file`'s status relative to it.
pub fn git_probe(dir: &Path, file: &str) -> GitInfo {
    let root = match run_git(dir, &["rev-parse", "--show-toplevel"]) {
        Ok(r) => r,
        Err(e) => {
            // "not a git repository" is the normal no-repo case; anything else
            // (missing binary, permissions) surfaces as unavailable.
            if e.contains("not a git repository") {
                return GitInfo {
                    repo: false,
                    root: None,
                    branch: None,
                    file_status: None,
                    last_commit: None,
                    unavailable: None,
                };
            }
            return GitInfo {
                repo: false,
                root: None,
                branch: None,
                file_status: None,
                last_commit: None,
                unavailable: Some(e),
            };
        }
    };
    let branch = {
        let b = run_git(dir, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default();
        if b.is_empty() {
            "（分离 HEAD）".to_string()
        } else {
            b
        }
    };
    let file_status = run_git(dir, &["status", "--porcelain", "--", file])
        .ok()
        .and_then(|out| porcelain_file_status(&out));
    let last_commit = run_git(dir, &["log", "-1", "--format=%h%x09%s%x09%ct", "--", file])
        .ok()
        .and_then(|out| {
            let mut parts = out.splitn(3, '\t');
            Some(LastCommit {
                hash: parts.next()?.to_string(),
                subject: parts.next()?.to_string(),
                time: parts.next()?.parse().ok()?,
            })
        });
    GitInfo {
        repo: true,
        root: Some(root),
        branch: Some(branch),
        file_status,
        last_commit,
        unavailable: None,
    }
}

/// `git init` in `dir`; returns the combined output for the toast.
pub fn git_init(dir: &Path) -> Result<String, String> {
    run_git(dir, &["init"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn porcelain_maps_status_columns() {
        assert_eq!(porcelain_file_status(""), None);
        assert_eq!(porcelain_file_status("\n \n"), None);
        assert_eq!(porcelain_file_status("?? new.log"), Some("未跟踪".into()));
        assert_eq!(
            porcelain_file_status("M  a.log"),
            Some("已暂存（修改）".into())
        );
        assert_eq!(
            porcelain_file_status("A  b.log"),
            Some("已暂存（新增）".into())
        );
        assert_eq!(
            porcelain_file_status(" M c.log"),
            Some("已修改（未暂存）".into())
        );
        assert_eq!(
            porcelain_file_status("MM d.log"),
            Some("已暂存（修改）".into())
        );
        assert_eq!(
            porcelain_file_status("!! e.log"),
            Some("已忽略".into())
        );
        assert_eq!(porcelain_file_status("ZZ f.log"), Some("状态 ZZ".into()));
    }

    #[test]
    fn probe_init_and_status_roundtrip() {
        if !git_available() {
            return; // CI/dev boxes without git skip the integration leg
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // no repo yet → repo=false, no "unavailable" noise
        let info = git_probe(dir, "a.log");
        assert!(!info.repo);
        assert!(info.unavailable.is_none());

        fs::write(dir.join("a.log"), "hello\n").unwrap();
        git_init(dir).unwrap();

        let info = git_probe(dir, "a.log");
        assert!(info.repo);
        assert!(!info.branch.as_deref().unwrap_or("").is_empty());
        assert_eq!(info.file_status.as_deref(), Some("未跟踪"));
        assert!(info.last_commit.is_none()); // no commit yet

        // a commit makes the file clean and fills last_commit
        run_git(dir, &["add", "."]).unwrap();
        run_git(dir, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "首提交"]).unwrap();
        let info = git_probe(dir, "a.log");
        assert_eq!(info.file_status, None);
        let lc = info.last_commit.unwrap();
        assert_eq!(lc.subject, "首提交");
        assert!(!lc.hash.is_empty());
    }

    #[test]
    fn probe_outside_a_repo_reports_unavailable_only_for_real_failures() {
        if !git_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("deep");
        fs::create_dir_all(&nested).unwrap();
        let info = git_probe(&nested, "x.txt");
        assert!(!info.repo && info.unavailable.is_none());
    }
}
