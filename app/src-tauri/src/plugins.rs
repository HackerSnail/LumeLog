//! Declarative syntax plugins (plugin interface, increment 1 — spec
//! `docs/specs/2026-10-30-plugin-syntax-design.md`). Plugins live in
//! `$APP_CONFIG/LumeLog/plugins/*.json`, one highlighting mode per file, and
//! carry NO executable code: extensions, an optional line-comment token,
//! keywords and ordered regex rules with a fixed tag vocabulary. This module
//! only reads and validates; tokenizing happens in the frontend
//! (`StreamLanguage`), where CodeMirror lives.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use rquickjs::FromJs;

use crate::dto::IpcError;

/// Upper bounds — a manifest is a config file, not a data set.
pub(crate) const MAX_PLUGIN_FILES: usize = 64;
pub(crate) const MAX_PLUGIN_BYTES: u64 = 256 * 1024;
pub(crate) const MAX_EXTENSIONS: usize = 16;
pub(crate) const MAX_KEYWORDS: usize = 512;
pub(crate) const MAX_RULES: usize = 32;
/// Parser plugins are source code (like scripts), not data: 1 MiB cap.
pub(crate) const MAX_PLUGIN_JS_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_COLUMNS: usize = 32;
/// Metadata loading executes the plugin once; a plugin that stalls its eval
/// must not multiply into a minutes-long listing.
const META_TIMEOUT: Duration = Duration::from_secs(5);
/// Whole-listing budget before remaining files are skipped.
const LIST_BUDGET: Duration = Duration::from_secs(15);

/// The tag vocabulary a rule may target. Kept identical to the tags the log/INI
/// stream modes use; the frontend maps them 1:1 onto CodeMirror stream tokens.
const TAGS: [&str; 7] = [
    "className",
    "keyword",
    "number",
    "string",
    "comment",
    "propertyName",
    "operator",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyntaxRuleJson {
    pub regex: String,
    pub tag: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyntaxPluginJson {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub line_comment: Option<String>,
    #[serde(default)]
    pub rules: Vec<SyntaxRuleJson>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyntaxPluginsReply {
    pub plugins: Vec<SyntaxPluginJson>,
    /// `<file>: <reason>` for every manifest that failed to load.
    pub errors: Vec<String>,
}

pub(crate) fn plugins_dir(app: &tauri::AppHandle) -> Result<PathBuf, IpcError> {
    let config = crate::commands::config_path(app)?;
    Ok(config
        .parent()
        .map(|p| p.join("plugins"))
        .unwrap_or_else(|| PathBuf::from("plugins")))
}

/// Read and validate every `*.json` under `dir`. One bad file never blocks the
/// rest: it becomes an `<file>: <reason>` entry in `errors`. Files are visited
/// in file-name order so duplicate ids resolve deterministically (first wins).
pub(crate) fn load_syntax_plugins(dir: &Path) -> (Vec<SyntaxPluginJson>, Vec<String>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![format!("读取插件目录失败: {e}")]),
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let (plugins, errors) = load_sorted(&files);
    let mut errors = errors;
    if files.len() > MAX_PLUGIN_FILES {
        errors.push(format!(
            "插件文件超过 {} 个，超出部分未加载",
            MAX_PLUGIN_FILES
        ));
    }
    (plugins, errors)
}

fn load_sorted(files: &[PathBuf]) -> (Vec<SyntaxPluginJson>, Vec<String>) {
    let mut plugins = Vec::new();
    let mut errors = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for path in files.iter().take(MAX_PLUGIN_FILES) {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match load_one(path) {
            Ok(p) => {
                if seen.contains(&p.id) {
                    errors.push(format!("{file}: 插件 id `{}` 重复，已跳过", p.id));
                } else {
                    seen.push(p.id.clone());
                    plugins.push(p);
                }
            }
            Err(reason) => errors.push(format!("{file}: {reason}")),
        }
    }
    (plugins, errors)
}

fn load_one(path: &Path) -> Result<SyntaxPluginJson, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读取失败: {e}"))?;
    if meta.len() > MAX_PLUGIN_BYTES {
        return Err(format!("文件过大（超过 {} 字节）", MAX_PLUGIN_BYTES));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;
    let plugin: SyntaxPluginJson =
        serde_json::from_str(&text).map_err(|e| format!("JSON 无效: {e}"))?;
    validate(&plugin)?;
    Ok(plugin)
}

/// Shared field rules for both plugin kinds (syntax JSON / parser JS).
pub(crate) fn check_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.chars().count() > 64 {
        return Err("id 长度须为 1–64".into());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("id 只允许字母、数字、`-`、`_`".into());
    }
    Ok(())
}

pub(crate) fn check_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("name 长度须为 1–64".into());
    }
    Ok(())
}

pub(crate) fn check_extension(ext: &str) -> Result<(), String> {
    if ext.is_empty() || ext.chars().count() > 16 {
        return Err(format!("扩展名 `{ext}` 长度须为 1–16"));
    }
    if !ext
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("扩展名 `{ext}` 含不允许的字符"));
    }
    Ok(())
}

/// Validate one syntax manifest (spec §2 of Plan 17). Returns a user-facing
/// reason on failure.
fn validate(p: &SyntaxPluginJson) -> Result<(), String> {
    check_id(&p.id)?;
    check_name(&p.name)?;
    if p.extensions.is_empty() || p.extensions.len() > MAX_EXTENSIONS {
        return Err(format!("extensions 须为 1–{MAX_EXTENSIONS} 个"));
    }
    for ext in &p.extensions {
        check_extension(ext)?;
    }
    if p.keywords.len() > MAX_KEYWORDS {
        return Err(format!("keywords 超过 {MAX_KEYWORDS} 个"));
    }
    for kw in &p.keywords {
        if kw.is_empty() || kw.chars().count() > 64 {
            return Err(format!("关键词 `{kw}` 长度须为 1–64"));
        }
        if kw.chars().any(|c| c.is_whitespace()) {
            return Err(format!("关键词 `{kw}` 不能含空白"));
        }
    }
    if let Some(lc) = &p.line_comment {
        if lc.is_empty() || lc.chars().count() > 8 || lc.chars().any(|c| c.is_whitespace()) {
            return Err("lineComment 须为 1–8 个非空白字符".into());
        }
    }
    if p.rules.len() > MAX_RULES {
        return Err(format!("rules 超过 {MAX_RULES} 条"));
    }
    for rule in &p.rules {
        if rule.regex.is_empty() || rule.regex.chars().count() > 512 {
            return Err("规则的 regex 长度须为 1–512".into());
        }
        if !TAGS.contains(&rule.tag.as_str()) {
            return Err(format!("规则的 tag `{}` 不在允许集内", rule.tag));
        }
    }
    Ok(())
}

/// IPC entry: list the syntax plugins under `$APP_CONFIG/LumeLog/plugins`.
#[tauri::command]
pub fn syntax_plugins_list(app: tauri::AppHandle) -> Result<SyntaxPluginsReply, IpcError> {
    let dir = plugins_dir(&app)?;
    let (plugins, errors) = load_syntax_plugins(&dir);
    Ok(SyntaxPluginsReply { plugins, errors })
}

// ---- parser plugins (increment 2: QuickJS `parseLine`) ----------------------

/// One table column a parser plugin emits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserColumnJson {
    pub key: String,
    pub label: String,
}

/// Metadata read by executing the plugin once in the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserPluginInfo {
    pub id: String,
    pub name: String,
    pub extensions: Vec<String>,
    pub columns: Vec<ParserColumnJson>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserPluginsReply {
    pub parsers: Vec<ParserPluginInfo>,
    pub errors: Vec<String>,
}

fn js_string<'js>(
    ctx: &rquickjs::Ctx<'js>,
    obj: &rquickjs::Object<'js>,
    field: &str,
) -> Result<String, String> {
    let v: rquickjs::Value = obj
        .get(field)
        .map_err(|e| format!("plugin.{field}: {e}"))?;
    if !v.is_string() {
        return Err(format!("plugin.{field} 必须是字符串"));
    }
    let s = rquickjs::String::from_js(ctx, v).map_err(|e| format!("plugin.{field}: {e}"))?;
    s.to_string().map_err(|e| format!("plugin.{field}: {e}"))
}

/// Execute `*.js` once and read the `plugin` metadata object. No `parseLine`
/// call happens here — this only validates shape so the panel can offer the
/// plugin without running row logic.
pub(crate) fn load_parser_metadata(path: &Path) -> Result<ParserPluginInfo, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("读取失败: {e}"))?;
    if meta.len() > MAX_PLUGIN_JS_BYTES {
        return Err(format!("文件过大（超过 {MAX_PLUGIN_JS_BYTES} 字节）"));
    }
    let code = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {e}"))?;

    let rt = rquickjs::Runtime::new().map_err(|e| format!("创建 JS 运行时失败：{e}"))?;
    rt.set_memory_limit(crate::script::MEMORY_LIMIT);
    rt.set_max_stack_size(crate::script::STACK_LIMIT);
    let start = Instant::now();
    rt.set_interrupt_handler(Some(Box::new(move || start.elapsed() > META_TIMEOUT)));
    let ctx = rquickjs::Context::full(&rt).map_err(|e| format!("创建 JS 上下文失败：{e}"))?;

    ctx.with(|ctx| -> Result<ParserPluginInfo, String> {
        use rquickjs::{Array, FromJs, Object, Value};
        // Metadata eval must not die on a stray console.* call.
        ctx.globals()
            .set(
                "__log",
                rquickjs::function::Func::from(|_level: String, _text: String| {}),
            )
            .map_err(|e| format!("注入 console 失败：{e}"))?;
        ctx.eval::<(), _>(crate::script::CONSOLE_SHIM)
            .map_err(|e| format!("执行失败：{}", crate::script::catch_message(&ctx, e)))?;
        ctx.eval::<(), _>(code.as_str())
            .map_err(|e| format!("执行失败：{}", crate::script::catch_message(&ctx, e)))?;

        let globals = ctx.globals();
        let value: Value = match globals.get::<_, Value>("plugin") {
            Ok(v) if !v.is_undefined() => v,
            _ => ctx
                .eval("typeof plugin === 'object' && plugin !== null ? plugin : undefined")
                .map_err(|e| format!("执行失败：{}", crate::script::catch_message(&ctx, e)))?,
        };
        // A top-level `const plugin` value must be copied out before the
        // context drops.
        let obj = Object::from_js(&ctx, value)
            .map_err(|_| "必须定义 plugin 对象".to_string())?;
        let parse_line: Value = obj
            .get("parseLine")
            .map_err(|e| format!("plugin.parseLine: {e}"))?;
        if !parse_line.is_function() {
            return Err("plugin.parseLine(line, ctx) 必须是函数".into());
        }

        let id = js_string(&ctx, &obj, "id")?;
        check_id(&id).map_err(|e| format!("plugin.id：{e}"))?;
        let name = js_string(&ctx, &obj, "name")?;
        check_name(&name).map_err(|e| format!("plugin.name：{e}"))?;

        let exts_val: Value = obj.get("extensions").map_err(|e| format!("plugin.extensions: {e}"))?;
        let exts = Array::from_js(&ctx, exts_val)
            .map_err(|_| "plugin.extensions 必须是数组".to_string())?;
        let mut extensions = Vec::new();
        for item in exts.iter::<Value>() {
            let item = item.map_err(|e| format!("plugin.extensions: {e}"))?;
            if !item.is_string() {
                return Err("plugin.extensions 必须是字符串数组".into());
            }
            let s = rquickjs::String::from_js(&ctx, item)
                .map_err(|e| format!("plugin.extensions: {e}"))?
                .to_string()
                .map_err(|e| format!("plugin.extensions: {e}"))?;
            check_extension(&s).map_err(|e| format!("plugin.extensions：{e}"))?;
            extensions.push(s);
        }
        if extensions.is_empty() || extensions.len() > MAX_EXTENSIONS {
            return Err(format!("plugin.extensions 须为 1–{MAX_EXTENSIONS} 个"));
        }

        let cols_val: Value = obj.get("columns").map_err(|e| format!("plugin.columns: {e}"))?;
        let cols =
            Array::from_js(&ctx, cols_val).map_err(|_| "plugin.columns 必须是数组".to_string())?;
        let mut columns = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for item in cols.iter::<Value>() {
            let item = item.map_err(|e| format!("plugin.columns: {e}"))?;
            let col = Object::from_js(&ctx, item)
                .map_err(|_| "plugin.columns 的元素必须是对象".to_string())?;
            let key = js_string(&ctx, &col, "key")
                .map_err(|e| format!("plugin.columns：{e}"))?;
            if key.is_empty() || key.chars().count() > 32 {
                return Err(format!("plugin.columns：列 key `{key}` 长度须为 1–32"));
            }
            if !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                return Err(format!("plugin.columns：列 key `{key}` 只允许字母、数字、`_`"));
            }
            if !seen.insert(key.clone()) {
                return Err(format!("plugin.columns：列 key `{key}` 重复"));
            }
            let label = js_string(&ctx, &col, "label")
                .map_err(|e| format!("plugin.columns：{e}"))?;
            if label.trim().is_empty() || label.chars().count() > 64 {
                return Err(format!("plugin.columns：列 `{key}` 的 label 长度须为 1–64"));
            }
            columns.push(ParserColumnJson { key, label });
        }
        if columns.is_empty() || columns.len() > MAX_COLUMNS {
            return Err(format!("plugin.columns 须为 1–{MAX_COLUMNS} 列"));
        }

        Ok(ParserPluginInfo {
            id,
            name,
            extensions,
            columns,
        })
    })
}

/// Load parser-plugin metadata for either kind: `.wasm` runs in the wasmi
/// host, everything else goes through the QuickJS path.
fn load_parser_plugin_any(path: &Path) -> Result<ParserPluginInfo, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "wasm" => {
            let bytes = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
            if bytes.len() as u64 > crate::wasmi_host::MAX_WASM_BYTES {
                return Err(format!(
                    "wasm 文件过大（超过 {} 字节）",
                    crate::wasmi_host::MAX_WASM_BYTES
                ));
            }
            let mut plugin = crate::wasmi_host::WasmPlugin::instantiate(&bytes)?;
            let info = plugin.meta()?;
            validate_parser_info(&info)?;
            Ok(info)
        }
        _ => load_parser_metadata(path),
    }
}

/// Shared shape checks for wasm `meta()` output (JS path validates inline).
fn validate_parser_info(info: &ParserPluginInfo) -> Result<(), String> {
    check_id(&info.id)?;
    check_name(&info.name)?;
    if info.extensions.is_empty() || info.extensions.len() > MAX_EXTENSIONS {
        return Err(format!("extensions 须为 1–{MAX_EXTENSIONS} 个"));
    }
    for ext in &info.extensions {
        check_extension(ext)?;
    }
    if info.columns.is_empty() || info.columns.len() > MAX_COLUMNS {
        return Err(format!("columns 须为 1–{MAX_COLUMNS} 列"));
    }
    for col in &info.columns {
        if col.key.is_empty() || col.key.chars().count() > 32 {
            return Err(format!("列 key `{}` 长度须为 1–32", col.key));
        }
        if !col
            .key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(format!("列 key `{}` 只允许字母、数字、`_`", col.key));
        }
        if col.label.trim().is_empty() || col.label.chars().count() > 64 {
            return Err(format!("列 `{}` 的 label 长度须为 1–64", col.key));
        }
    }
    Ok(())
}

/// Walk `dir` for `*.js` / `*.wasm` parser plugins (file-name order, one bad
/// file never blocks the rest, first id wins). A stalled eval/instantiation is
/// bounded by per-file fuel/timeouts and the whole listing by `LIST_BUDGET`.
pub(crate) fn list_parser_plugins(dir: &Path) -> (Vec<ParserPluginInfo>, Vec<String>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![format!("读取插件目录失败: {e}")]),
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|x| x == "js" || x == "wasm")
        })
        .collect();
    files.sort();
    let mut parsers = Vec::new();
    let mut errors = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let started = Instant::now();
    for path in files.iter().take(MAX_PLUGIN_FILES) {
        if started.elapsed() > LIST_BUDGET {
            errors.push("插件加载超预算，其余文件已跳过".into());
            break;
        }
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match load_parser_plugin_any(path) {
            Ok(info) => {
                if seen.contains(&info.id) {
                    errors.push(format!("{file}: 插件 id `{}` 重复，已跳过", info.id));
                } else {
                    seen.push(info.id.clone());
                    parsers.push(info);
                }
            }
            Err(reason) => errors.push(format!("{file}: {reason}")),
        }
    }
    (parsers, errors)
}

/// IPC entry: list parser plugins (executes each `*.js` once in the sandbox).
#[tauri::command]
pub fn parser_plugins_list(app: tauri::AppHandle) -> Result<ParserPluginsReply, IpcError> {
    let dir = plugins_dir(&app)?;
    let (parsers, errors) = list_parser_plugins(&dir);
    Ok(ParserPluginsReply { parsers, errors })
}

/// Locate a parser plugin by id (never trust a path from the frontend).
pub(crate) fn find_parser_plugin(dir: &Path, plugin_id: &str) -> Result<PathBuf, IpcError> {
    let (parsers, _errors) = list_parser_plugins(dir);
    if parsers.iter().any(|p| p.id == plugin_id) {
        // Metadata was validated for this id; resolve the file by re-walking in
        // the same (sorted) order the listing used.
        let rd = std::fs::read_dir(dir)
            .map_err(|e| IpcError::new("io", format!("读取插件目录失败: {e}")))?;
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| x == "js" || x == "wasm")
            })
            .collect();
        files.sort();
        for path in files {
            if let Ok(meta) = load_parser_plugin_any(&path) {
                if meta.id == plugin_id {
                    return Ok(path);
                }
            }
        }
    }
    Err(IpcError::new(
        "plugin_not_found",
        format!("解析插件 `{plugin_id}` 不存在"),
    ))
}

/// Rows per IPC frame (`can_parse`'s 512/batch policy).
pub(crate) const PARSER_BATCH: usize = 512;
/// Total wall-clock budget for one parse run (script engine parity).
pub(crate) const RUN_SECONDS: u64 = crate::script::MAX_SECONDS;
/// One cell that huge would wreck the IPC frame more than help anyone.
const MAX_CELL_CHARS: usize = 4096;

/// One parsed row: the 1-based source line (editor jump) plus the cell strings
/// in the plugin's declared column order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserRowJson {
    pub line: u64,
    pub cells: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserStats {
    pub rows: u64,
    pub lines: u64,
    pub skipped: u64,
    pub cancelled: bool,
    pub error: Option<String>,
}

/// Row-object → cells conversion lives in JS so `String(number)` keeps JS
/// formatting; `null` marks a value type the table cannot show (warned once).
const CELLS_SHIM: &str = r#"
globalThis.__cells = function (row, keys) {
  var out = new Array(keys.length);
  for (var i = 0; i < keys.length; i++) {
    var v = row[keys[i]];
    if (v === undefined || v === null) out[i] = "";
    else if (typeof v === "string") out[i] = v;
    else if (typeof v === "number" || typeof v === "boolean") out[i] = String(v);
    else out[i] = null;
  }
  return out;
};
"#;

enum ParseStop {
    Js(String),
    Cancelled,
}

/// Execute `plugin.parseLine(line, ctx)` over the whole document, streaming
/// `ParserRowJson` batches. Runs in its own sandbox (script-engine limits);
/// the plugin file is loaded and metadata re-validated here so a run never
/// depends on a previous listing.
pub(crate) fn run_parser_plugin(
    doc: Arc<Mutex<lumelog_core::doc::DocFile>>,
    plugin_path: &Path,
    on_batch: &mut dyn FnMut(Vec<ParserRowJson>),
    on_console: &mut dyn FnMut(crate::script::ScriptConsole),
    cancel: Arc<dyn Fn() -> bool + Send + Sync>,
) -> ParserStats {
    use rquickjs::{Array, Context, Function, Object, Runtime, Value};
    use rquickjs::function::Func;
    use crate::commands::DocLines;

    let mut stats = ParserStats::default();
    if let Ok(meta) = std::fs::metadata(plugin_path) {
        if meta.len() > MAX_PLUGIN_JS_BYTES {
            stats.error = Some(format!("插件文件过大（超过 {MAX_PLUGIN_JS_BYTES} 字节）"));
            return stats;
        }
    }
    let code = match std::fs::read_to_string(plugin_path) {
        Ok(c) => c,
        Err(e) => {
            stats.error = Some(format!("读取插件失败：{e}"));
            return stats;
        }
    };
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
            return stats;
        }
    };
    rt.set_memory_limit(crate::script::MEMORY_LIMIT);
    rt.set_max_stack_size(crate::script::STACK_LIMIT);
    let start = Instant::now();
    let deadline = Duration::from_secs(RUN_SECONDS);
    {
        let cancel = cancel.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            cancel() || start.elapsed() > deadline
        })));
    }
    let ctx = match Context::full(&rt) {
        Ok(c) => c,
        Err(e) => {
            stats.error = Some(format!("创建 JS 上下文失败：{e}"));
            return stats;
        }
    };

    let console_pending: std::rc::Rc<std::cell::RefCell<Vec<crate::script::ScriptConsole>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let console_buf = std::rc::Rc::new(std::cell::RefCell::new(crate::script::ConsoleBuf::new()));
    let mut cancelled = false;
    let mut script_error: Option<String> = None;

    let result = ctx.with(|ctx| -> Result<(), ParseStop> {
        let globals = ctx.globals();
        let log = {
            let pending = console_pending.clone();
            let buf = console_buf.clone();
            Func::from(move |level: String, text: String| {
                if let Some(msg) = buf.borrow_mut().push(&level, text) {
                    pending.borrow_mut().push(msg);
                }
            })
        };
        globals
            .set("__log", log)
            .map_err(|e| ParseStop::Js(format!("注入 console 失败：{e}")))?;
        ctx.eval::<(), _>(crate::script::CONSOLE_SHIM)
            .map_err(|e| ParseStop::Js(crate::script::catch_message(&ctx, e)))?;
        ctx.eval::<(), _>(CELLS_SHIM)
            .map_err(|e| ParseStop::Js(crate::script::catch_message(&ctx, e)))?;
        ctx.eval::<(), _>(code.as_str())
            .map_err(|e| ParseStop::Js(format!("插件错误：{}", crate::script::catch_message(&ctx, e))))?;

        let value: Value = match globals.get::<_, Value>("plugin") {
            Ok(v) if !v.is_undefined() => v,
            _ => ctx
                .eval("typeof plugin === 'object' && plugin !== null ? plugin : undefined")
                .map_err(|e| ParseStop::Js(crate::script::catch_message(&ctx, e)))?,
        };
        let obj =
            Object::from_js(&ctx, value).map_err(|_| ParseStop::Js("必须定义 plugin 对象".into()))?;
        let parse_line: Value = obj
            .get("parseLine")
            .map_err(|e| ParseStop::Js(format!("plugin.parseLine: {e}")))?;
        if !parse_line.is_function() {
            return Err(ParseStop::Js("plugin.parseLine(line, ctx) 必须是函数".into()));
        }
        let parse_line = Function::from_js(&ctx, parse_line)
            .map_err(|_| ParseStop::Js("plugin.parseLine(line, ctx) 必须是函数".into()))?;

        let columns = read_columns(&ctx, &obj)?;
        let keys = Array::new(ctx.clone())
            .map_err(|e| ParseStop::Js(format!("构建列参数失败：{e}")))?;
        for (i, col) in columns.iter().enumerate() {
            keys.set(i, col.key.as_str())
                .map_err(|e| ParseStop::Js(format!("构建列参数失败：{e}")))?;
        }
        let keys_value = keys.clone().into_value();
        let cells_fn: Function = globals
            .get("__cells")
            .map_err(|e| ParseStop::Js(format!("内部转换器缺失：{e}")))?;

        let mut batch: Vec<ParserRowJson> = Vec::new();
        let mut warned_unsupported = false;
        for (line_number, text) in lines.by_ref() {
            if cancel() {
                cancelled = true;
                break;
            }
            if start.elapsed() > deadline {
                script_error = Some(format!("解析超时（{RUN_SECONDS}s）"));
                break;
            }
            stats.lines += 1;
            let ctx_obj = Object::new(ctx.clone())
                .map_err(|e| ParseStop::Js(format!("创建 ctx 失败：{e}")))?;
            ctx_obj
                .set("lineNumber", line_number as f64)
                .map_err(|e| ParseStop::Js(format!("设置 ctx.lineNumber 失败：{e}")))?;
            ctx_obj
                .set("index", line_number.saturating_sub(1) as f64)
                .map_err(|e| ParseStop::Js(format!("设置 ctx.index 失败：{e}")))?;
            ctx_obj
                .set("path", source_path.as_str())
                .map_err(|e| ParseStop::Js(format!("设置 ctx.path 失败：{e}")))?;

            let row: Value = parse_line.call((text, ctx_obj)).map_err(|e| {
                if cancel() {
                    ParseStop::Cancelled
                } else if start.elapsed() > deadline {
                    ParseStop::Js(format!("解析超时（{RUN_SECONDS}s）"))
                } else {
                    ParseStop::Js(format!(
                        "第 {line_number} 行：{}",
                        crate::script::catch_message(&ctx, e)
                    ))
                }
            })?;
            if row.is_null() || row.is_undefined() {
                stats.skipped += 1;
            } else if row.is_object() {
                let cells_val: Value = cells_fn.call((row, keys_value.clone())).map_err(|e| {
                    if cancel() {
                        ParseStop::Cancelled
                    } else {
                        ParseStop::Js(format!(
                            "第 {line_number} 行：{}",
                            crate::script::catch_message(&ctx, e)
                        ))
                    }
                })?;
                let cells_arr = Array::from_js(&ctx, cells_val)
                    .map_err(|_| ParseStop::Js("内部转换器返回异常".into()))?;
                let mut cells = Vec::with_capacity(columns.len());
                for cv in cells_arr.iter::<Value>() {
                    let cv = cv.map_err(|e| ParseStop::Js(format!("第 {line_number} 行：{e}")))?;
                    if cv.is_string() {
                        let mut s = rquickjs::String::from_js(&ctx, cv)
                            .map_err(|e| ParseStop::Js(format!("第 {line_number} 行：{e}")))?
                            .to_string()
                            .map_err(|e| ParseStop::Js(format!("第 {line_number} 行：{e}")))?;
                        if s.chars().count() > MAX_CELL_CHARS {
                            s = s.chars().take(MAX_CELL_CHARS).collect();
                        }
                        cells.push(s);
                    } else {
                        // __cells only emits null for unsupported value types.
                        if !warned_unsupported {
                            warned_unsupported = true;
                            on_console(crate::script::ScriptConsole {
                                level: "warn".into(),
                                text: "parseLine 返回的列值类型不受支持，已置空".into(),
                            });
                        }
                        cells.push(String::new());
                    }
                }
                stats.rows += 1;
                batch.push(ParserRowJson {
                    line: line_number,
                    cells,
                });
                if batch.len() >= PARSER_BATCH {
                    on_batch(std::mem::take(&mut batch));
                }
            } else {
                return Err(ParseStop::Js(format!(
                    "第 {line_number} 行：parseLine 必须返回对象、null 或 undefined"
                )));
            }

            let msgs: Vec<_> = console_pending.borrow_mut().drain(..).collect();
            for m in msgs {
                on_console(m);
            }
        }
        if !batch.is_empty() {
            on_batch(std::mem::take(&mut batch));
        }
        Ok(())
    });

    let msgs: Vec<_> = console_pending.borrow_mut().drain(..).collect();
    for m in msgs {
        on_console(m);
    }
    if let Err(stop) = result {
        match stop {
            ParseStop::Js(msg) => script_error = Some(msg),
            ParseStop::Cancelled => cancelled = true,
        }
    }
    stats.cancelled = cancelled;
    stats.error = script_error;
    stats
}

/// Re-read `plugin.columns` inside the run context (shape re-validated).
fn read_columns<'js>(
    ctx: &rquickjs::Ctx<'js>,
    obj: &rquickjs::Object<'js>,
) -> Result<Vec<ParserColumnJson>, ParseStop> {
    use rquickjs::{Array, FromJs, Object, Value};
    let cols_val: Value = obj
        .get("columns")
        .map_err(|e| ParseStop::Js(format!("plugin.columns: {e}")))?;
    let cols =
        Array::from_js(ctx, cols_val).map_err(|_| ParseStop::Js("plugin.columns 必须是数组".into()))?;
    let mut columns = Vec::new();
    for item in cols.iter::<Value>() {
        let item = item.map_err(|e| ParseStop::Js(format!("plugin.columns: {e}")))?;
        let col = Object::from_js(ctx, item)
            .map_err(|_| ParseStop::Js("plugin.columns 的元素必须是对象".into()))?;
        let key: Value = col.get("key").map_err(|e| ParseStop::Js(format!("plugin.columns: {e}")))?;
        let key = if key.is_string() {
            rquickjs::String::from_js(ctx, key)
                .map_err(|e| ParseStop::Js(format!("plugin.columns: {e}")))?
                .to_string()
                .map_err(|e| ParseStop::Js(format!("plugin.columns: {e}")))?
        } else {
            return Err(ParseStop::Js("plugin.columns 的 key 必须是字符串".into()));
        };
        columns.push(ParserColumnJson {
            key,
            label: String::new(), // labels were validated at listing time; runs only need keys
        });
    }
    if columns.is_empty() || columns.len() > MAX_COLUMNS {
        return Err(ParseStop::Js(format!(
            "plugin.columns 须为 1–{MAX_COLUMNS} 列"
        )));
    }
    Ok(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lumelog-plugins-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const VALID: &str = r##"{
        "id": "plc-log",
        "name": "PLC 日志",
        "extensions": ["plc", "plclog"],
        "lineComment": "#",
        "keywords": ["START", "STOP"],
        "rules": [
            { "regex": "^\\d{2}:\\d{2}", "tag": "className" },
            { "regex": "0x[0-9a-fA-F]+", "tag": "number" }
        ]
    }"##;

    #[test]
    fn missing_dir_yields_empty() {
        let (plugins, errors) = load_syntax_plugins(&temp_dir("missing").join("nope"));
        assert!(plugins.is_empty());
        assert!(errors.is_empty());
        std::fs::remove_dir_all(temp_dir("missing")).ok();
    }

    #[test]
    fn valid_manifest_loads() {
        let dir = temp_dir("valid");
        write(&dir, "plc.json", VALID);
        let (plugins, errors) = load_syntax_plugins(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "plc-log");
        assert_eq!(plugins[0].extensions, vec!["plc", "plclog"]);
        assert_eq!(plugins[0].line_comment.as_deref(), Some("#"));
        assert_eq!(plugins[0].rules.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bad_tag_is_rejected() {
        let dir = temp_dir("badtag");
        write(&dir, "a.json", VALID.replace("\"className\"", "\"colorful\"").as_str());
        let (plugins, errors) = load_syntax_plugins(&dir);
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("a.json:"), "{errors:?}");
        assert!(errors[0].contains("tag"), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_field_is_rejected() {
        let dir = temp_dir("unknown");
        write(
            &dir,
            "a.json",
            VALID.replace("\"keywords\"", "\"keywrds\"").as_str(),
        );
        let (_, errors) = load_syntax_plugins(&dir);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown field") || errors[0].contains("JSON"), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn broken_json_does_not_block_others() {
        let dir = temp_dir("mixed");
        write(&dir, "a-broken.json", "{ not json");
        write(&dir, "b-good.json", VALID);
        let (plugins, errors) = load_syntax_plugins(&dir);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "plc-log");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("a-broken.json:"), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_id_first_file_wins() {
        let dir = temp_dir("dup");
        write(&dir, "a.json", VALID);
        write(&dir, "b.json", VALID.replace("\"PLC 日志\"", "\"副本\"").as_str());
        let (plugins, errors) = load_syntax_plugins(&dir);
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "PLC 日志");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("b.json:"), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn limits_are_enforced() {
        let dir = temp_dir("limits");
        // extensions overflow
        let exts: Vec<String> = (0..17).map(|i| format!("e{i}")).collect();
        write(
            &dir,
            "ext.json",
            VALID.replace("[\"plc\", \"plclog\"]", &serde_json::to_string(&exts).unwrap()).as_str(),
        );
        let (_, errors) = load_syntax_plugins(&dir);
        assert!(errors.iter().any(|e| e.contains("extensions")), "{errors:?}");
        // whitespace in keyword
        write(&dir, "kw.json", VALID.replace("\"START\", \"STOP\"", "\"A B\"").as_str());
        let (_, errors) = load_syntax_plugins(&dir);
        assert!(errors.iter().any(|e| e.contains("空白")), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_json_files_are_ignored() {
        let dir = temp_dir("ext");
        write(&dir, "readme.txt", "hello");
        write(&dir, "plc.json", VALID);
        let (plugins, errors) = load_syntax_plugins(&dir);
        assert_eq!(plugins.len(), 1);
        assert!(errors.is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- parser plugins (increment 2) --------------------------------------

    const PARSER_OK: &str = r##"
        var plugin = {
            id: "myfmt",
            name: "My 格式",
            extensions: ["mylog"],
            columns: [
                { key: "time", label: "时间" },
                { key: "level", label: "级别" },
            ],
            parseLine(line, ctx) { return null; },
        };
    "##;

    #[test]
    fn parser_metadata_loads() {
        let dir = temp_dir("pmeta");
        write(&dir, "myfmt.js", PARSER_OK);
        write(&dir, "syntax.json", VALID); // syntax plugins stay out of this list
        let (parsers, errors) = list_parser_plugins(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(parsers.len(), 1);
        assert_eq!(parsers[0].id, "myfmt");
        assert_eq!(parsers[0].columns.len(), 2);
        assert_eq!(parsers[0].columns[0].key, "time");
        assert_eq!(parsers[0].extensions, vec!["mylog"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parser_metadata_rejects_bad_shapes() {
        let dir = temp_dir("pbad");
        // no plugin object at all
        write(&dir, "a.js", "var x = 1;\n");
        // parseLine missing
        write(&dir, "b.js", r##"var plugin = { id: "b", name: "b", extensions: ["b"], columns: [{key:"k",label:"K"}] };"##);
        // columns empty
        write(&dir, "c.js", r##"var plugin = { id: "c", name: "c", extensions: ["c"], columns: [], parseLine() { return null; } };"##);
        // duplicate column key
        write(&dir, "d.js", r##"var plugin = { id: "d", name: "d", extensions: ["d"], columns: [{key:"k",label:"K"},{key:"k",label:"K2"}], parseLine() { return null; } };"##);
        let (parsers, errors) = list_parser_plugins(&dir);
        assert!(parsers.is_empty());
        assert_eq!(errors.len(), 4, "{errors:?}");
        assert!(errors.iter().all(|e| e.contains(".js:")), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parser_top_level_const_is_found() {
        let dir = temp_dir("pconst");
        write(
            &dir,
            "c.js",
            r##"const plugin = { id: "cfmt", name: "CF", extensions: ["cf"], columns: [{key:"k",label:"K"}], parseLine(line) { return {k: line}; } };"##,
        );
        let (parsers, errors) = list_parser_plugins(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(parsers.len(), 1);
        assert_eq!(parsers[0].id, "cfmt");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parser_broken_js_does_not_block_others() {
        let dir = temp_dir("pmixed");
        write(&dir, "a-broken.js", "this is not(");
        write(&dir, "b-good.js", PARSER_OK);
        let (parsers, errors) = list_parser_plugins(&dir);
        assert_eq!(parsers.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("a-broken.js:"), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parser_duplicate_id_first_wins() {
        let dir = temp_dir("pdup");
        write(&dir, "a.js", PARSER_OK);
        write(
            &dir,
            "b.js",
            &PARSER_OK.replace("\"My 格式\"", "\"副本\""),
        );
        let (parsers, errors) = list_parser_plugins(&dir);
        assert_eq!(parsers.len(), 1);
        assert_eq!(parsers[0].name, "My 格式");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("b.js:"), "{errors:?}");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- parser run (increment 2) -------------------------------------------

    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};

    const PARSER_RUN: &str = r##"
        var plugin = {
            id: "runfmt",
            name: "Run",
            extensions: ["rlog"],
            columns: [
                { key: "word", label: "W" },
                { key: "num", label: "N" },
                { key: "bad", label: "B" },
            ],
            parseLine(line, ctx) {
                if (line === "skip") return null;
                var parts = line.split(" ");
                return { word: parts[0], num: ctx.lineNumber, bad: { obj: true } };
            },
        };
    "##;

    fn open_doc(path: &std::path::Path) -> Arc<Mutex<lumelog_core::doc::DocFile>> {
        Arc::new(Mutex::new(
            lumelog_core::doc::DocFile::open(path, 1 << 20).unwrap(),
        ))
    }

    #[test]
    fn parser_run_end_to_end() {
        let dir = temp_dir("prun");
        let doc_path = dir.join("in.rlog");
        std::fs::write(&doc_path, "START 1\nskip\nSTOP 2\n").unwrap();
        write(&dir, "p.js", PARSER_RUN);

        let batches: RefCell<Vec<Vec<ParserRowJson>>> = RefCell::new(Vec::new());
        let consoles: RefCell<Vec<crate::script::ScriptConsole>> = RefCell::new(Vec::new());
        let cancel: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(|| false);
        let stats = run_parser_plugin(
            open_doc(&doc_path),
            &dir.join("p.js"),
            &mut |rows| batches.borrow_mut().push(rows),
            &mut |m| consoles.borrow_mut().push(m),
            cancel,
        );
        assert_eq!(stats.error, None, "{stats:?}");
        assert_eq!(stats.lines, 3);
        assert_eq!(stats.rows, 2);
        assert_eq!(stats.skipped, 1);
        assert!(!stats.cancelled);

        let all: Vec<ParserRowJson> = batches.into_inner().into_iter().flatten().collect();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].line, 1);
        assert_eq!(all[0].cells, vec!["START", "1", ""]); // number -> JS String()
        assert_eq!(all[1].line, 3);
        assert_eq!(all[1].cells[0], "STOP");
        // the object-valued column collapses to "" with a one-time warning
        assert!(consoles
            .into_inner()
            .iter()
            .any(|m| m.level == "warn" && m.text.contains("不受支持")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parser_run_honours_cancel() {
        let dir = temp_dir("pcancel");
        let doc_path = dir.join("in.rlog");
        std::fs::write(&doc_path, "a\nb\nc\nd\n").unwrap();
        write(
            &dir,
            "p.js",
            r#"var plugin = { id: "p", name: "P", extensions: ["rlog"], columns: [{key:"k",label:"K"}], parseLine(line) { return { k: line }; } };"#,
        );
        // 1st check (loop) passes, 2nd (post-call) passes, 3rd cancels: exactly
        // one row lands before the run stops.
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let cancel: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || {
            let n = calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            n >= 2
        });
        let batches: RefCell<Vec<Vec<ParserRowJson>>> = RefCell::new(Vec::new());
        let stats = run_parser_plugin(
            open_doc(&doc_path),
            &dir.join("p.js"),
            &mut |rows| batches.borrow_mut().push(rows),
            &mut |_| {},
            cancel,
        );
        assert!(stats.cancelled, "{stats:?}");
        assert!(stats.error.is_none());
        let all: Vec<ParserRowJson> = batches.into_inner().into_iter().flatten().collect();
        assert_eq!(all.len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parser_run_surfaces_console_and_js_errors() {
        let dir = temp_dir("perr");
        let doc_path = dir.join("in.rlog");
        std::fs::write(&doc_path, "x\n").unwrap();
        write(
            &dir,
            "p.js",
            r#"var plugin = { id: "p", name: "P", extensions: ["rlog"], columns: [{key:"k",label:"K"}], parseLine(line) { console.log("hi", line); if (line === "x") throw new Error("boom"); return {k: line}; } };"#,
        );
        let consoles: RefCell<Vec<crate::script::ScriptConsole>> = RefCell::new(Vec::new());
        let stats = run_parser_plugin(
            open_doc(&doc_path),
            &dir.join("p.js"),
            &mut |_| {},
            &mut |m| consoles.borrow_mut().push(m),
            Arc::new(|| false),
        );
        assert!(stats.error.as_deref().unwrap_or("").contains("boom"), "{stats:?}");
        assert!(consoles
            .into_inner()
            .iter()
            .any(|m| m.level == "log" && m.text == "hi x"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn find_parser_plugin_by_id() {
        let dir = temp_dir("pfind");
        write(&dir, "p.js", PARSER_RUN);
        let path = find_parser_plugin(&dir, "runfmt").unwrap();
        assert!(path.file_name().unwrap() == "p.js");
        let err = find_parser_plugin(&dir, "nope").unwrap_err();
        assert_eq!(err.code, "plugin_not_found");
        std::fs::remove_dir_all(&dir).ok();
    }
}
