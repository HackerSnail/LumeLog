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
use std::sync::atomic::{AtomicUsize, Ordering};
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
    ///
    /// v1 uses one knob for both pre-write confirmations (the plan pins a single
    /// `overwrite_policy` field). Because the two gates are coupled, `save_text`
    /// computes the unmappable count *before* the conflict gate and returns it
    /// inside `CoreError::Conflict`, so the UI can fold both into ONE *informed*
    /// confirmation — the user is told exactly what a forced overwrite costs (the
    /// external change *and* how many characters become `?`) before consenting.
    /// `Force` therefore means "the user accepted every gate `SafetyConfirm`
    /// would raise, with full knowledge of each"; it must never be the default.
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
    /// Characters that could not be represented in the target encoding, each
    /// written as a single `?` placeholder (`encode_all`; spec §4.4 confirm flow).
    pub replace_count: usize,
    /// True when a conflict was detected but `Force` overrode it.
    pub conflict: bool,
}

/// Detect the dominant line-ending style of `text`.
///
/// Both `CRLF` and lone `LF` present → [`EolStyle::Mixed`]; only one → that
/// style; no newline at all → [`EolStyle::Lf`] (a safe default for writing).
///
/// A lone `\r` (CR-only line ending) is treated as content, not a break, and so
/// never drives detection — an accepted v1 simplification (the plan's `detect_eol`
/// contract only counts CRLF vs LF); such files normalize to `Lf` on save.
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

/// Whether `a` and `b` name the same existing file. Lexical equality is not
/// enough: on case-insensitive or link/alias-heavy filesystems a Save-As path
/// can be a different spelling of the file we loaded, and treating it as "a new
/// path" would silently skip the conflict check. Canonicalizing both (when they
/// exist) collapses those spellings; if either is absent we fall back to the
/// lexical comparison (a brand-new destination is genuinely not the original).
fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}

/// Encode + write `text` to `opts`' target. Returns [`CoreError::Conflict`] or
/// [`CoreError::Unmappable`] (before touching the disk) when the policy requires
/// confirmation; `Force` proceeds through both. On a successful *in-place* save
/// the doc's handle and line index are rebuilt from the bytes just written, so
/// later `chunk_at`/search calls reflect the saved content (not stale pre-save
/// data). A Save-As to another path leaves `doc` bound to its original file.
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
    let overwriting_original = match opts.path.as_ref() {
        None => true,
        Some(p) => same_file(p, doc.path()),
    };
    // Write through symlinks and land the temp file in the real target's
    // directory (rename must stay on the same volume to be atomic).
    let real_target = fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
    let current = disk_state(&real_target);
    let conflict = overwriting_original && current != Some(disk_meta);

    // Encode *before* the conflict gate so a conflict response can also report
    // the unmappable consequence of forcing. `encode_all` is pure (no disk side
    // effect), so this does not weaken the "leave the disk untouched until
    // confirmed" guarantee. The plan pins one `overwrite_policy` knob, so the UI
    // folds both gates into a single informed confirmation instead of two.
    let lf = normalize_lf(text);
    let with_eol = apply_eol(&lf, opts.eol);
    let enc = encode_all(opts.encoding, &with_eol);

    if conflict && opts.overwrite_policy == OverwritePolicy::SafetyConfirm {
        // Hand back the *live* disk meta + the unmappable count so the UI can
        // spell out exactly what a forced overwrite will do.
        return Err(CoreError::Conflict {
            disk: current.unwrap_or((0, SystemTime::UNIX_EPOCH)),
            unmappable: enc.replace_count,
        });
    }

    if enc.replace_count > 0 && opts.overwrite_policy != OverwritePolicy::Force {
        // Unmappable characters would be written as a `?` placeholder — confirm
        // first, and crucially leave the disk untouched (spec §4.4).
        return Err(CoreError::Unmappable(enc.replace_count));
    }

    atomic_write(&real_target, &enc.bytes)?;

    // Rebind the doc only when we replaced the very file it reads. A Save-As
    // must not repoint `doc`'s size/mtime/index at a different file (that would
    // poison the next conflict baseline). `reload_after_save` recomputes the
    // applied EOL from the real bytes, so `Mixed` correctly settles to `Crlf`.
    if overwriting_original {
        doc.reload_after_save(opts.encoding)?;
    }

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
    // Everything up to the rename touches only the temp file: if any step fails
    // we remove it, so the target is never left half-written or the directory
    // littered (spec §4.4 atomicity).
    if let Err(e) = write_stage(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    match fs::rename(&tmp, target) {
        Ok(()) => {
            // Durability of the rename itself: on unix the directory entry needs
            // an fsync too, or a crash can lose (or zero-length) the swap.
            sync_dir(target.parent());
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            if is_locked(&e) {
                return Err(CoreError::Locked);
            }
            Err(CoreError::Io(e))
        }
    }
}

/// Write `bytes` to `tmp` and fsync it. Split out so `atomic_write` can clean
/// the temp file on any failure before the rename commits.
fn write_stage(tmp: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    let mut f = fs::File::create(tmp)?;
    f.write_all(bytes)?;
    f.flush()?;
    f.sync_all()?;
    Ok(())
}

/// fsync a directory so a rename/create is durable on unix. No-op elsewhere
/// (Windows flushes metadata through the rename).
fn sync_dir(dir: Option<&Path>) {
    #[cfg(unix)]
    if let Some(d) = dir {
        if let Ok(f) = fs::File::open(d) {
            let _ = f.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
}

fn temp_path(target: &Path) -> PathBuf {
    let stem = target
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "doc".to_string());
    // Uniquify with pid + a per-process counter: two files in the same directory
    // that share a stem (`a.txt` and `a.md`), or two tabs saving concurrently,
    // must not fight over one `.{stem}.lumelogtmp` and clobber each other.
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    target.with_file_name(format!(".{stem}.{}.{}.lumelogtmp", std::process::id(), seq))
}

/// A rename failure meaning "the destination is held by another process", so the
/// UI should suggest saving elsewhere. This downgrade is Windows-specific: a
/// rename over a file another handle has open fails with ERROR_ACCESS_DENIED (5)
/// or ERROR_SHARING_VIOLATION (32). On unix a `PermissionDenied` is a genuine
/// permissions problem (read-only / unwritable dir), not a lock, so it stays an
/// `Io` error rather than misleading the user with "file is in use".
fn is_locked(e: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        e.kind() == std::io::ErrorKind::PermissionDenied
            || matches!(e.raw_os_error(), Some(5) | Some(32))
    }
    #[cfg(not(windows))]
    {
        let _ = e;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::decode_all;

    fn write_tmp(bytes: &[u8]) -> PathBuf {
        // Each call gets its own directory so parallel tests never observe each
        // other's intermediate `.lumelogtmp` files when scanning for strays.
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!("lumelog_save_{}_{id}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("doc.log");
        fs::write(&p, bytes).unwrap();
        p
    }

    fn parent(path: &Path) -> &Path {
        path.parent().expect("test files live in a directory")
    }

    /// Names of leftover `*.lumelogtmp` files in `dir` (atomic-write litter).
    fn stray_tmps(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".lumelogtmp"))
                    .collect()
            })
            .unwrap_or_default()
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
        assert!(matches!(err, CoreError::Conflict { .. }), "got {err:?}");
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
        assert!(stray_tmps(parent(&p)).is_empty(), "no temp file created before the gate");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn conflict_reports_unmappable_count_too() {
        // Final-review I-1: the plan pins ONE overwrite knob, so a conflict
        // response must ALSO carry the lossy-encode consequence — letting the UI
        // fold both gates into a single *informed* force confirmation (spec §4.4)
        // instead of forcing the user to accept an irreversible `?`-write blind.
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let stale = doc.disk_meta();
        // External rewrite trips the conflict gate.
        fs::write(&p, b"changed by someone else\n").unwrap();

        let err = save_text(
            &mut doc,
            "ok💩\n",
            &opts(None, EncodingLabel::Gbk, EolStyle::Lf),
            stale,
        )
        .unwrap_err();
        match err {
            CoreError::Conflict { unmappable, .. } => {
                assert_eq!(unmappable, 1, "conflict carries the lossy-encode count");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(
            fs::read(&p).unwrap(),
            b"changed by someone else\n",
            "disk untouched until confirmed"
        );
        assert!(stray_tmps(parent(&p)).is_empty(), "no temp file before the gate");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn unmappable_force_writes_placeholder_not_numeric_ref() {
        // C3a/C3b: a Force save past the unmappable gate must land a single
        // placeholder char (GBK has no U+FFFD, so `?`), never encoding_rs' `&#128169;`.
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        let mut force = opts(None, EncodingLabel::Gbk, EolStyle::Lf);
        force.overwrite_policy = OverwritePolicy::Force;
        let rep = save_text(&mut doc, "ok💩\n", &force, meta).unwrap();
        assert_eq!(rep.replace_count, 1);
        let saved = fs::read(&p).unwrap();
        assert!(!saved.windows(2).any(|w| w == b"&#"), "no numeric ref on disk");
        assert_eq!(decode_all(EncodingLabel::Gbk, &saved).text, "ok?\n");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn atomic_write_leaves_no_tmp_on_success() {
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        save_text(&mut doc, "hello\n", &opts(None, EncodingLabel::Utf8, EolStyle::Lf), meta).unwrap();
        assert!(stray_tmps(parent(&p)).is_empty(), "temp file was renamed away");
        // C1: after an in-place save the doc's handle + index must be rebound, so
        // `chunk_at` returns the freshly written content, not the pre-save bytes.
        assert_eq!(doc.chunk_at(0, 1).unwrap().text, "hello\n", "post-save read is fresh");
        // The recorded meta now matches what we wrote, so a follow-up save is
        // not a false conflict.
        let meta2 = doc.disk_meta();
        let rep = save_text(&mut doc, "hello2\n", &opts(None, EncodingLabel::Utf8, EolStyle::Lf), meta2).unwrap();
        assert!(!rep.conflict, "reload refreshed the baseline");
        assert_eq!(doc.chunk_at(0, 1).unwrap().text, "hello2\n");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn save_inplace_reindexes_after_transcode() {
        // In-place save under a different encoding rebinds the doc to the new
        // bytes/encoding, so subsequent reads decode correctly (not stale UTF-8).
        let p = write_tmp(b"abc\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        save_text(&mut doc, "台架\n", &opts(None, EncodingLabel::Gbk, EolStyle::Crlf), meta).unwrap();
        assert_eq!(doc.encoding(), EncodingLabel::Gbk, "doc follows the saved encoding");
        assert_eq!(doc.eol_style(), EolStyle::Crlf, "doc records the applied EOL");
        assert_eq!(doc.chunk_at(0, 1).unwrap().text, "台架\r\n");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn save_mixed_and_lone_cr_normalizes_to_crlf() {
        // Mixed is written as CRLF; a lone CR becomes an LF first (Review Focus 3
        // + the `normalize_lf` second replace branch, which the other tests miss).
        let p = write_tmp(b"x\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        save_text(&mut doc, "a\r\nb\rc\n", &opts(None, EncodingLabel::Utf8, EolStyle::Mixed), meta).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"a\r\nb\r\nc\r\n");
        // M1: what actually landed is pure CRLF, so the doc settles to Crlf.
        assert_eq!(doc.eol_style(), EolStyle::Crlf, "Mixed normalizes to Crlf");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn save_bom_roundtrip_utf8bom() {
        // BOM must be written back by encode_all and survive a reopen+re-detect.
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        save_text(&mut doc, "héllo\n", &opts(None, EncodingLabel::Utf8Bom, EolStyle::Lf), meta).unwrap();
        let saved = fs::read(&p).unwrap();
        assert!(saved.starts_with(&[0xEF, 0xBB, 0xBF]), "leading BOM written");
        let reopened = DocFile::open(&p, 1 << 20).unwrap();
        assert_eq!(reopened.encoding(), EncodingLabel::Utf8Bom, "BOM re-detected");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn rename_failure_removes_tmp() {
        // I2 (rename branch): a rename that fails after the temp file was written
        // (here the target is a directory) must clean the temp file up.
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        let dir = parent(&p).join("target_dir");
        fs::create_dir_all(&dir).unwrap();
        let res = save_text(&mut doc, "x\n", &opts(Some(dir.clone()), EncodingLabel::Utf8, EolStyle::Lf), meta);
        assert!(res.is_err(), "renaming a file over a directory must fail");
        assert!(stray_tmps(&dir).is_empty(), "temp cleaned after rename failure");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn write_stage_failure_returns_error_and_creates_nothing() {
        // I2 (write branch): if the temp file cannot even be created (its parent
        // is missing), save errors out before the rename and produces no target.
        let p = write_tmp(b"seed\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let meta = doc.disk_meta();
        let missing = parent(&p).join("no_such_dir").join("out.log");
        let res = save_text(&mut doc, "x\n", &opts(Some(missing.clone()), EncodingLabel::Utf8, EolStyle::Lf), meta);
        assert!(res.is_err(), "writing into a missing directory must fail");
        assert!(!missing.exists(), "no target file created");
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
        assert_eq!(fs::read(&p).unwrap(), b"someone else\n", "original file untouched");
        // I1: a Save-As must leave `doc` bound to its ORIGINAL file — its
        // recorded meta must not be repointed at the destination.
        assert_eq!(doc.disk_meta(), stale, "Save-As must not repoint the doc baseline");
        fs::remove_file(&p).ok();
        fs::remove_file(&out).ok();
    }

    #[test]
    fn save_as_to_nonexistent_path_creates_it_without_conflict() {
        // C2 / canonicalize fallback: the destination does not exist yet, so
        // canonicalize fails; we must still create it and never mistake the
        // original's dirtied baseline for a conflict on the fresh destination.
        let p = write_tmp(b"original\n");
        let mut doc = DocFile::open(&p, 1 << 20).unwrap();
        let stale = doc.disk_meta();
        fs::write(&p, b"external change\n").unwrap(); // dirty the original (irrelevant here)
        let out = parent(&p).join("brand_new.log"); // deliberately NOT pre-created
        assert!(!out.exists());
        let rep = save_text(&mut doc, "copy\n", &opts(Some(out.clone()), EncodingLabel::Utf8, EolStyle::Lf), stale).unwrap();
        assert!(!rep.conflict, "a fresh destination is never a conflict");
        assert_eq!(fs::read(&out).unwrap(), b"copy\n");
        assert_eq!(doc.disk_meta(), stale, "Save-As leaves the doc bound to the original");
        fs::remove_file(&p).ok();
        fs::remove_file(&out).ok();
    }
}
