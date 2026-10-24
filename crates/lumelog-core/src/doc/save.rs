//! Save path: line-ending detection/expansion, conflict + unmappable gates, and
//! an atomic write (temp file → fsync → rename), spec §4.4.
//!
//! `save_text` is a *pure* disk operation driven by the frontend's current
//! document text (CodeMirror keeps LF internally; EOL only matters at save
//! time). Two decisions gate the write and, per spec §4.4, must be confirmed
//! by the user before proceeding — both surface as errors so the UI can ask,
//! then retry with [`OverwritePolicy::Force`]:
//! - the file on disk changed since open (`CoreError::Conflict`),
//! - some characters are unrepresentable in the target encoding
//!   (`CoreError::Unmappable`).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::doc::file::DocFile;
use crate::encoding::{encode_all, EncodingLabel};
use crate::error::CoreError;

/// Dominant line-ending style of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EolStyle {
    Lf,
    Crlf,
    /// Both styles present; on save treated as [`EolStyle::Crlf`] (spec §4.4
    /// one-click normalize), while still reported so the status bar can prompt.
    Mixed,
}

/// How to treat a save when the on-disk file is not what we loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwritePolicy {
    /// Refuse to overwrite on conflict / unmappable and return an error so the
    /// UI can confirm with the user (spec §4.4 "确认后执行").
    SafetyConfirm,
    /// The user already confirmed — write regardless of conflict / unmappable.
    Force,
}

/// Save parameters. `path: None` means overwrite the document's original path.
pub struct SaveOptions {
    pub path: Option<PathBuf>,
    pub encoding: EncodingLabel,
    /// Line-ending style to write with. `Mixed` is written as CRLF.
    pub eol: EolStyle,
    pub overwrite_policy: OverwritePolicy,
}

/// Outcome of a completed write.
#[derive(Debug)]
pub struct SaveReport {
    /// Characters replaced with U+FFFD in the target encoding.
    pub replace_count: usize,
    /// True when a conflict was detected but `Force` overrode it.
    pub conflict: bool,
}

/// Detect the dominant line-ending style of `text`.
///
/// Both `CRLF` and lone `LF` present → [`EolStyle::Mixed`]; only one → that
/// style; no newline at all → [`EolStyle::Lf`] (a safe default for writing).
pub fn detect_eol(text: &str) -> EolStyle {
    let b = text.as_bytes();
    let (mut crlf, mut lf) = (0usize, 0usize);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' && i + 1 < b.len() && b[i + 1] == b'\n' {
            crlf += 1;
            i += 2;
        } else if b[i] == b'\n' {
            lf += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    match (crlf > 0, lf > 0) {
        (true, true) => EolStyle::Mixed,
        (true, false) => EolStyle::Crlf,
        (false, true) => EolStyle::Lf,
        (false, false) => EolStyle::Lf,
    }
}

/// Normalize every line ending to LF (CRLF and lone CR both become LF).
fn normalize_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Expand an LF-normalized text to the target style (`Mixed` → CRLF).
fn apply_eol(lf_text: &str, eol: EolStyle) -> String {
    match eol {
        EolStyle::Lf => lf_text.to_string(),
        EolStyle::Crlf | EolStyle::Mixed => lf_text.replace('\n', "\r\n"),
    }
}

/// Current on-disk `(size, mtime)` for `path`, or `None` if it is absent.
fn disk_state(path: &Path) -> Option<(u64, SystemTime)> {
    let m = fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()?))
}

/// Encode + write `text` to `opts`' target. Returns [`CoreError::Conflict`] or
/// [`CoreError::Unmappable`] (before touching the disk) when the policy requires
/// confirmation; `Force` proceeds through both. On success updates `doc`'s
/// recorded size/mtime/eol so the next conflict check is against what we wrote.
pub fn save_text(
    doc: &mut DocFile,
    text: &str,
    opts: &SaveOptions,
    disk_meta: (u64, SystemTime),
) -> Result<SaveReport, CoreError> {
    let target = opts
        .path
        .clone()
        .unwrap_or_else(|| doc.path().to_path_buf());

    // Conflict is only meaningful when overwriting the file we loaded; a Save-As
    // to a fresh path has nothing to conflict with.
    let overwriting_original = opts.path.as_ref().map_or(true, |p| p == doc.path());
    let current = disk_state(&target);
    let conflict = overwriting_original && current != Some(disk_meta);
    if conflict && opts.overwrite_policy == OverwritePolicy::SafetyConfirm {
        // Hand back the *live* disk meta so the UI can show what changed.
        return Err(CoreError::Conflict(current.unwrap_or((0, SystemTime::UNIX_EPOCH))));
    }

    let lf = normalize_lf(text);
    let with_eol = apply_eol(&lf, opts.eol);
    let enc = encode_all(opts.encoding, &with_eol);

    if enc.replace_count > 0 && opts.overwrite_policy != OverwritePolicy::Force {
        // Unmappable characters would be written as U+FFFD — confirm first, and
        // crucially leave the disk untouched (spec §4.4).
        return Err(CoreError::Unmappable(enc.replace_count));
    }

    atomic_write(&target, &enc.bytes)?;

    // Re-read the metadata we just produced so `doc.disk_meta()` reflects reality.
    let (size, mtime) = disk_state(&target)
        .ok_or_else(|| CoreError::Io(std::io::Error::other("target vanished after write")))?;
    doc.note_saved(size, mtime, opts.eol);

    Ok(SaveReport {
        replace_count: enc.replace_count,
        conflict,
    })
}

/// Write `bytes` to a sibling temp file, fsync it, then atomically rename over
/// `target`. A rename blocked by another process holding `target` open (the
/// Windows sharing-violation case) maps to [`CoreError::Locked`]; the UI then
/// offers "save elsewhere" (spec §4.4). The temp file is removed on any failure.
fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let tmp = temp_path(target);
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        f.sync_all()?;
    }
    match fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Best-effort cleanup so we never leave a stray temp file behind.
            let _ = fs::remove_file(&tmp);
            if is_locked(&e) {
                return Err(CoreError::Locked);
            }
            Err(CoreError::Io(e))
        }
    }
}

fn temp_path(target: &Path) -> PathBuf {
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "doc".to_string());
    target.with_file_name(format!(".{stem}.lumelogtmp"))
}

/// A rename failure caused by the destination being open/locked by another
/// process (Windows sharing violation 32 / access denied 5), or the portable
/// `PermissionDenied` kind.
fn is_locked(e: &std::io::Error) -> bool {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    matches!(e.raw_os_error(), Some(5) | Some(32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::decode_all;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn write_tmp(bytes: &[u8]) -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut p = std::env::temp_dir();
        p.push(format!("lumelog_save_{}_{id}.log", std::process::id()));
        fs::write(&p, bytes).unwrap();
        p
    }

    fn opts(path: Option<PathBuf>, encoding: EncodingLabel, eol: EolStyle) -> SaveOptions {
        SaveOptions {
            path,
            encoding,
            eol,
            overwrite_policy: OverwritePolicy::SafetyConfirm,
        }
    }

    #[test]
    fn detect_eol_cases() {
        assert_eq!(detect_eol("a\r\nb\r\n"), EolStyle::Crlf);
        assert_eq!(detect_eol("a\nb\nc"), EolStyle::Lf);
        assert_eq!(detect_eol("a\r\nb\nc"), EolStyle::Mixed);
        assert_eq!(detect_eol("no newline"), EolStyle::Lf);
    }

    #[test]
    fn save_roundtrip_gbk_crlf() {
        // Review Focus 3: a Chinese document saved as GBK + CRLF must re-decode
        // to the same text with every line ending in \r\n.
        let p = write_tmp(b"x\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let text = "台架1\n数据2\n";
        let meta = doc.disk_meta();
        let rep = save_text(&mut doc, text, &opts(None, EncodingLabel::Gbk, EolStyle::Crlf), meta).unwrap();
        assert_eq!(rep.replace_count, 0, "all chars representable in GBK");
        assert!(!rep.conflict);

        let saved = fs::read(&p).unwrap();
        let dec = decode_all(EncodingLabel::Gbk, &saved);
        assert_eq!(dec.invalid_count, 0);
        assert_eq!(dec.text, "台架1\r\n数据2\r\n", "each line expanded to CRLF");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn conflict_blocks_save_without_force() {
        let p = write_tmp(b"original\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let stale = doc.disk_meta();
        // External process rewrites the file after we opened it.
        fs::write(&p, b"changed by someone else and longer\n").unwrap();

        let err = save_text(&mut doc, "mine\n", &opts(None, EncodingLabel::Utf8, EolStyle::Lf), stale)
            .unwrap_err();
        assert!(matches!(err, CoreError::Conflict(_)), "got {err:?}");
        assert_eq!(fs::read(&p).unwrap(), b"changed by someone else and longer\n", "not overwritten");

        // Force overrides the conflict and reports it.
        let mut force = opts(None, EncodingLabel::Utf8, EolStyle::Lf);
        force.overwrite_policy = OverwritePolicy::Force;
        let rep = save_text(&mut doc, "mine\n", &force, stale).unwrap();
        assert!(rep.conflict, "Force proceeds but flags the conflict");
        assert_eq!(fs::read(&p).unwrap(), b"mine\n");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn unmappable_reports_count_before_write() {
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let before = fs::read(&p).unwrap();
        let meta = doc.disk_meta();
        let err = save_text(
            &mut doc,
            "ok💩\n",
            &opts(None, EncodingLabel::Gbk, EolStyle::Lf),
            meta,
        )
        .unwrap_err();
        assert!(matches!(err, CoreError::Unmappable(1)), "got {err:?}");
        assert_eq!(fs::read(&p).unwrap(), before, "disk untouched on unmappable");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn atomic_write_leaves_no_tmp_on_success() {
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        save_text(&mut doc, "hello\n", &opts(None, EncodingLabel::Utf8, EolStyle::Lf), meta).unwrap();
        assert!(!temp_path(&p).exists(), "temp file was renamed away");
        // doc's recorded meta now matches what we wrote, so a follow-up save is
        // not a false conflict.
        let meta2 = doc.disk_meta();
        let rep = save_text(&mut doc, "hello2\n", &opts(None, EncodingLabel::Utf8, EolStyle::Lf), meta2).unwrap();
        assert!(!rep.conflict, "note_saved refreshed the baseline");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn save_as_to_new_path_is_not_a_conflict() {
        let p = write_tmp(b"original\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let stale = doc.disk_meta();
        // Externally dirty the original — irrelevant when saving elsewhere.
        fs::write(&p, b"someone else\n").unwrap();
        let out = write_tmp(b"");
        let rep = save_text(&mut doc, "copy\n", &opts(Some(out.clone()), EncodingLabel::Utf8, EolStyle::Lf), stale).unwrap();
        assert!(!rep.conflict, "Save-As never conflicts on the destination");
        assert_eq!(fs::read(&out).unwrap(), b"copy\n");
        assert_eq!(fs::read(&p).unwrap(), b"someone else\n", "original untouched");
        fs::remove_file(&p).ok();
        fs::remove_file(&out).ok();
    }
}
