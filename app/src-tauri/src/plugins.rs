//! Declarative syntax plugins (plugin interface, increment 1 — spec
//! `docs/specs/2026-10-30-plugin-syntax-design.md`). Plugins live in
//! `$APP_CONFIG/LumeLog/plugins/*.json`, one highlighting mode per file, and
//! carry NO executable code: extensions, an optional line-comment token,
//! keywords and ordered regex rules with a fixed tag vocabulary. This module
//! only reads and validates; tokenizing happens in the frontend
//! (`StreamLanguage`), where CodeMirror lives.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dto::IpcError;

/// Upper bounds — a manifest is a config file, not a data set.
pub(crate) const MAX_PLUGIN_FILES: usize = 64;
pub(crate) const MAX_PLUGIN_BYTES: u64 = 256 * 1024;
pub(crate) const MAX_EXTENSIONS: usize = 16;
pub(crate) const MAX_KEYWORDS: usize = 512;
pub(crate) const MAX_RULES: usize = 32;

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

/// Validate one manifest (spec §2). Returns a user-facing reason on failure.
fn validate(p: &SyntaxPluginJson) -> Result<(), String> {
    if p.id.is_empty() || p.id.chars().count() > 64 {
        return Err("id 长度须为 1–64".into());
    }
    if !p.id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("id 只允许字母、数字、`-`、`_`".into());
    }
    let name = p.name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("name 长度须为 1–64".into());
    }
    if p.extensions.is_empty() || p.extensions.len() > MAX_EXTENSIONS {
        return Err(format!("extensions 须为 1–{MAX_EXTENSIONS} 个"));
    }
    for ext in &p.extensions {
        if ext.is_empty() || ext.chars().count() > 16 {
            return Err(format!("扩展名 `{ext}` 长度须为 1–16"));
        }
        if !ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("扩展名 `{ext}` 含不允许的字符"));
        }
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
}
