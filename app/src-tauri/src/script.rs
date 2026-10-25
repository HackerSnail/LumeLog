//! User scripts (Roadmap-3). Scripts live in `$APP_CONFIG/LumeLog/scripts` and
//! define `onLine(line, ctx)`; the runner (Task 2) executes them in a locked-down
//! QuickJS runtime over the active document's lines. This module owns the store
//! (list/read/write/delete), name validation and the bounded console buffer.

use std::cell::RefCell;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use lumelog_core::doc::DocFile;
use rquickjs::function::Func;
use rquickjs::{Context, FromJs, Function, Object, Runtime, Value};
use serde::Serialize;
use tauri::AppHandle;

use crate::commands::DocLines;
use crate::dto::IpcError;

pub(crate) const MAX_CONSOLE_MESSAGES: usize = 200;
pub(crate) const MAX_CONSOLE_CHARS: usize = 4096;
/// Scripts are source code, not logs: refuse to slurp anything oversized.
pub(crate) const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub(crate) const MAX_SECONDS: u64 = 300;
pub(crate) const PROGRESS_EVERY: u64 = 100_000;
const MEMORY_LIMIT: usize = 64 * 1024 * 1024;
const STACK_LIMIT: usize = 1024 * 1024;

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
    let path = dir.join(name);
    let meta = std::fs::metadata(&path).map_err(|e| IpcError::new("io", format!("read script: {e}")))?;
    if meta.len() > MAX_SCRIPT_BYTES {
        return Err(IpcError::new("invalid_argument", "脚本过大（超过 1 MiB）"));
    }
    let bytes = std::fs::read(&path).map_err(|e| IpcError::new("io", format!("read script: {e}")))?;
    // Scripts may be saved by an external editor as UTF-8 (with or without BOM)
    // or a legacy ANSI code page; decode leniently rather than failing.
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(body).into_owned())
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

/// `console.*` shim: formats arguments (objects via JSON) and forwards one
/// string to the host `__log`. Formatting lives in JS so no value marshalling is
/// needed in Rust.
const CONSOLE_SHIM: &str = r#"
(function () {
  const emit = globalThis.__log;
  const fmt = (v) => {
    try { return typeof v === "string" ? v : JSON.stringify(v); }
    catch (e) { return String(v); }
  };
  const make = (level) => (...a) => emit(level, a.map(fmt).join(" "));
  globalThis.console = { log: make("log"), info: make("info"), warn: make("warn"), error: make("error") };
  delete globalThis.__log;
})();
"#;

enum RunStop {
    /// Script-level error: keep the written prefix at the target.
    Js(String),
    /// Output I/O failure: discard the tmp file.
    Io(String),
    /// The user cancelled while JS was executing (not an error).
    Cancelled,
}

fn catch_message(ctx: &rquickjs::Ctx<'_>, err: rquickjs::Error) -> String {
    let caught = ctx.catch();
    if caught.is_object() {
        if let Ok(obj) = Object::from_js(ctx, caught.clone()) {
            if let Some(ex) = rquickjs::Exception::from_object(obj) {
                if let Some(msg) = ex.message() {
                    return msg;
                }
            }
        }
    }
    if let Ok(s) = rquickjs::String::from_js(ctx, caught) {
        if let Ok(msg) = s.to_string() {
            return msg;
        }
    }
    err.to_string()
}

/// Run `code` over the document's lines. Output is UTF-8 + `\n`; a successful
/// (or script-error) run renames the tmp file over `target`, so the caller sees
/// exactly the emitted prefix.
#[allow(clippy::too_many_arguments)] // doc/code/target/callbacks/cancel
pub fn run_script(
    doc: Arc<Mutex<DocFile>>,
    code: &str,
    target: &Path,
    on_console: &mut dyn FnMut(ScriptConsole),
    on_progress: &mut dyn FnMut(ScriptProgress),
    cancel: Arc<dyn Fn() -> bool + Send + Sync>,
) -> ScriptStats {
    let mut stats = ScriptStats::default();
    let tmp = target.with_file_name(format!(
        ".{}.tmp-{}-{}",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("script-out"),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let file = match File::create(&tmp) {
        Ok(f) => f,
        Err(e) => {
            stats.error = Some(format!("创建输出文件失败：{e}"));
            return stats;
        }
    };
    let mut writer = BufWriter::new(file);
    let source_path = doc
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .path()
        .to_string_lossy()
        .into_owned();
    let mut lines = DocLines::new(doc);

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            stats.error = Some(format!("创建 JS 运行时失败：{e}"));
            let _ = std::fs::remove_file(&tmp);
            return stats;
        }
    };
    rt.set_memory_limit(MEMORY_LIMIT);
    rt.set_max_stack_size(STACK_LIMIT);
    let start = Instant::now();
    let deadline = Duration::from_secs(MAX_SECONDS);
    {
        let cancel = cancel.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            cancel() || start.elapsed() > deadline
        })));
    }

    let console_pending: Rc<RefCell<Vec<ScriptConsole>>> = Rc::new(RefCell::new(Vec::new()));
    let console_buf = Rc::new(RefCell::new(ConsoleBuf::new()));
    let mut cancelled = false;
    let mut script_error: Option<String> = None;

    let ctx = match Context::full(&rt) {
        Ok(c) => c,
        Err(e) => {
            stats.error = Some(format!("创建 JS 上下文失败：{e}"));
            let _ = std::fs::remove_file(&tmp);
            return stats;
        }
    };

    let result = ctx.with(|ctx| -> Result<(), RunStop> {
        let globals = ctx.globals();
        let log = {
            let pending = console_pending.clone();
            let buf = console_buf.clone();
            Func::from(
                move |level: String, text: String| {
                    if let Some(msg) = buf.borrow_mut().push(&level, text) {
                        pending.borrow_mut().push(msg);
                    }
                },
            )
        };
        globals
            .set("__log", log)
            .map_err(|e| RunStop::Js(format!("注入 console 失败：{e}")))?;
        ctx.eval::<(), _>(CONSOLE_SHIM)
            .map_err(|e| RunStop::Js(catch_message(&ctx, e)))?;
        if let Err(e) = ctx.eval::<(), _>(code) {
            return Err(RunStop::Js(format!(
                "脚本错误：{}",
                catch_message(&ctx, e)
            )));
        }
        // A top-level `const/let onLine` lives in the global lexical scope, not
        // on `globalThis`, so a property lookup alone would reject modern style.
        let on_line: Function = match globals.get("onLine") {
            Ok(f) => f,
            Err(_) => {
                let v: Value = ctx
                    .eval("typeof onLine === 'function' ? onLine : undefined")
                    .map_err(|e| RunStop::Js(catch_message(&ctx, e)))?;
                Function::from_js(&ctx, v).map_err(|_| {
                    RunStop::Js("脚本必须定义 onLine(line, ctx)".into())
                })?
            }
        };

        let mut out_bytes: u64 = 0;
        for (line_number, text) in lines.by_ref() {
            if cancel() {
                cancelled = true;
                break;
            }
            if start.elapsed() > deadline {
                script_error = Some(format!("脚本超时（{MAX_SECONDS}s）"));
                break;
            }
            stats.scanned += 1;
            let ctx_obj = Object::new(ctx.clone())
                .map_err(|e| RunStop::Js(format!("创建 ctx 失败：{e}")))?;
            ctx_obj
                .set("lineNumber", line_number as f64)
                .map_err(|e| RunStop::Js(format!("设置 ctx.lineNumber 失败：{e}")))?;
            ctx_obj
                .set("index", line_number.saturating_sub(1) as f64)
                .map_err(|e| RunStop::Js(format!("设置 ctx.index 失败：{e}")))?;
            ctx_obj
                .set("path", source_path.as_str())
                .map_err(|e| RunStop::Js(format!("设置 ctx.path 失败：{e}")))?;
            let value: Value = on_line.call((text, ctx_obj)).map_err(|e| {
                if cancel() {
                    RunStop::Cancelled
                } else if start.elapsed() > deadline {
                    RunStop::Js(format!("脚本超时（{MAX_SECONDS}s）"))
                } else {
                    RunStop::Js(format!(
                        "第 {line_number} 行：{}",
                        catch_message(&ctx, e)
                    ))
                }
            })?;
            if value.is_null() || value.is_undefined() {
                stats.dropped += 1;
            } else if value.is_string() {
                let s = rquickjs::String::from_js(&ctx, value)
                    .map_err(|e| RunStop::Js(format!("第 {line_number} 行：{e}")))?;
                let s = s
                    .to_string()
                    .map_err(|e| RunStop::Js(format!("第 {line_number} 行：{e}")))?;
                let bytes = s.len() as u64 + 1;
                if out_bytes + bytes > MAX_OUTPUT_BYTES {
                    script_error = Some("输出超过 4 GiB 上限".into());
                    break;
                }
            if writer
                .write_all(s.as_bytes())
                .and_then(|_| writer.write_all(b"\n"))
                .is_err()
            {
                return Err(RunStop::Io("写入输出文件失败".into()));
            }
                out_bytes += bytes;
                stats.written += 1;
            } else {
                return Err(RunStop::Js(format!(
                    "第 {line_number} 行：onLine 必须返回 string、null 或 undefined"
                )));
            }

            let msgs: Vec<_> = console_pending.borrow_mut().drain(..).collect();
            for m in msgs {
                on_console(m);
            }
            if stats.scanned % PROGRESS_EVERY == 0 {
                on_progress(ScriptProgress {
                    scanned: stats.scanned,
                    written: stats.written,
                });
            }
        }
        Ok(())
    });

    // Forward any console lines produced by the final/erroring call.
    let msgs: Vec<_> = console_pending.borrow_mut().drain(..).collect();
    for m in msgs {
        on_console(m);
    }
    stats.truncated_console = console_buf.borrow().truncated();
    on_progress(ScriptProgress {
        scanned: stats.scanned,
        written: stats.written,
    });

    if let Err(stop) = result {
        match stop {
            RunStop::Js(msg) => script_error = Some(msg),
            RunStop::Cancelled => cancelled = true,
            RunStop::Io(msg) => {
                let _ = std::fs::remove_file(&tmp);
                stats.cancelled = cancelled;
                stats.error = Some(msg);
                return stats;
            }
        }
    }
    stats.cancelled = cancelled;
    if let Err(e) = writer.flush() {
        let _ = std::fs::remove_file(&tmp);
        stats.cancelled = cancelled;
        stats.error = Some(format!("写入输出文件失败：{e}"));
        return stats;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        stats.error = Some(format!("保存输出失败：{e}"));
        return stats;
    }
    stats.error = script_error;
    stats
}

#[cfg(test)]
mod run_tests {
    use super::*;

    fn run_case(
        name: &str,
        doc_text: &str,
        code: &str,
    ) -> (ScriptStats, String, Vec<ScriptConsole>) {
        let dir = std::env::temp_dir().join(format!(
            "lumelog-script-run-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let doc_path = dir.join("in.log");
        std::fs::write(&doc_path, doc_text).unwrap();
        let doc = Arc::new(Mutex::new(DocFile::open(&doc_path, 1 << 20).unwrap()));
        let target = dir.join("out.txt");
        let mut msgs = Vec::new();
        let mut no_progress = |_p: ScriptProgress| {};
        let stats = run_script(
            doc,
            code,
            &target,
            &mut |m| msgs.push(m),
            &mut no_progress,
            Arc::new(|| false),
        );
        let out = std::fs::read_to_string(&target).unwrap_or_default();
        std::fs::remove_dir_all(&dir).ok();
        (stats, out, msgs)
    }

    #[test]
    fn keep_replace_drop() {
        let (stats, out, _) = run_case(
            "threeways",
            "a\n\nb\n",
            "function onLine(line) { if (line === '') return null; return line.toUpperCase(); }",
        );
        assert_eq!(out, "A\nB\n");
        assert_eq!(stats.scanned, 3, "DocLines drops the trailing terminator");
        assert_eq!(stats.written, 2);
        assert_eq!(stats.dropped, 1);
        assert!(stats.error.is_none());
    }

    #[test]
    fn ctx_line_numbers() {
        let (stats, out, _) = run_case(
            "ctx",
            "a\nb\nc",
            "function onLine(line, ctx) { return String(ctx.lineNumber) + ':' + line; }",
        );
        assert_eq!(out, "1:a\n2:b\n3:c\n");
        assert_eq!(stats.written, 3);
    }

    #[test]
    fn js_throw_stops_with_prefix() {
        let (stats, out, _) = run_case(
            "throw",
            "a\nb\nc",
            "function onLine(line, ctx) { if (ctx.lineNumber === 2) throw new Error('boom'); return line; }",
        );
        assert_eq!(out, "a\n", "prefix before the error is kept");
        let err = stats.error.unwrap();
        assert!(err.contains("第 2 行"), "{err}");
        assert!(err.contains("boom"), "{err}");
    }

    #[test]
    fn missing_on_line_errors() {
        let (stats, out, _) = run_case("noentry", "a\n", "const x = 1;");
        assert_eq!(out, "");
        assert!(stats.error.unwrap().contains("onLine"));
    }

    #[test]
    fn bad_return_type_errors() {
        let (stats, out, _) = run_case("badret", "a\n", "function onLine() { return 42; }");
        assert_eq!(out, "");
        assert!(stats.error.unwrap().contains("必须返回"));
    }

    #[test]
    fn utf8_lf_bytes() {
        let (stats, out, _) = run_case(
            "utf8",
            "中文",
            "function onLine(line) { return line; }",
        );
        assert_eq!(out.as_bytes(), "中文\n".as_bytes());
        assert_eq!(stats.written, 1);
    }

    #[test]
    fn console_capture_and_formatting() {
        let (_stats, _out, msgs) = run_case(
            "console",
            "a\n",
            "function onLine(line) { console.log('hi', {a: 1}); console.warn('w'); console.error('e'); return line; }",
        );
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].level, "log");
        assert!(msgs[0].text.contains("hi"), "{}", msgs[0].text);
        assert!(msgs[0].text.contains("\"a\":1"), "{}", msgs[0].text);
        assert_eq!(msgs[1].level, "warn");
        assert_eq!(msgs[2].level, "error");
    }

    #[test]
    fn const_arrow_on_line_is_found() {
        let (stats, out, _) = run_case("constfn", "a\nb", "const onLine = (l) => l.toUpperCase();");
        assert_eq!(out, "A\nB\n");
        assert!(stats.error.is_none());
    }

    #[test]
    fn successful_run_leaves_no_tmp() {
        let dir = std::env::temp_dir().join(format!(
            "lumelog-script-tmp-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let doc_path = dir.join("in.log");
        std::fs::write(&doc_path, "a\n").unwrap();
        let doc = Arc::new(Mutex::new(DocFile::open(&doc_path, 1 << 20).unwrap()));
        let target = dir.join("out.txt");
        let mut none = |_m: ScriptConsole| {};
        let mut nop = |_p: ScriptProgress| {};
        let stats = run_script(
            doc,
            "function onLine(l) { return l; }",
            &target,
            &mut none,
            &mut nop,
            Arc::new(|| false),
        );
        assert!(stats.error.is_none());
        let tmp: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(tmp.is_empty());
        std::fs::remove_dir_all(&dir).ok();
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
    fn stats_camel_case() {
        let v = serde_json::to_value(ScriptStats {
            scanned: 2,
            written: 1,
            dropped: 1,
            cancelled: false,
            truncated_console: true,
            error: None,
        })
        .unwrap();
        assert_eq!(v["scanned"], 2);
        assert_eq!(v["truncatedConsole"], true);
        assert!(v["error"].is_null());
    }

    #[test]
    fn read_strips_bom_and_survives_legacy_encoding() {
        let dir = temp_dir("readenc");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("function onLine(){ return 1; }".as_bytes());
        std::fs::write(dir.join("bom.js"), &bytes).unwrap();
        assert!(read_script_in(&dir, "bom.js").unwrap().starts_with("function"));
        std::fs::write(
            dir.join("gbk.js"),
            [0xC0, 0xCF, b'a'], // 非 UTF-8 字节（如 GBK 保存）
        )
        .unwrap();
        assert!(read_script_in(&dir, "gbk.js").unwrap().contains('a'));
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
