//! Declarative syntax plugins (plugin interface, increment 1 — spec
//! `docs/specs/2026-10-30-plugin-syntax-design.md`). Plugins live in
//! `$APP_CONFIG/LumeLog/plugins/*.json`, one highlighting mode per file, and
//! carry NO executable code: extensions, an optional line-comment token,
//! keywords and ordered regex rules with a fixed tag vocabulary. This module
//! only reads and validates; tokenizing happens in the frontend
//! (`StreamLanguage`), where CodeMirror lives.

use std::path::{Path, PathBuf};
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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserColumnJson {
    pub key: String,
    pub label: String,
}

/// Metadata read by executing the plugin once in the sandbox.
#[derive(Debug, Clone, Serialize)]
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

/// Walk `dir` for `*.js` parser plugins (file-name order, one bad file never
/// blocks the rest, first id wins). A stalled eval is bounded by `META_TIMEOUT`
/// per file and the whole listing by `LIST_BUDGET`.
pub(crate) fn list_parser_plugins(dir: &Path) -> (Vec<ParserPluginInfo>, Vec<String>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), Vec::new()),
        Err(e) => return (Vec::new(), vec![format!("读取插件目录失败: {e}")]),
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "js"))
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
        match load_parser_metadata(path) {
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
}
