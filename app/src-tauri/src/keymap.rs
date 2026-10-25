//! Keymap override store (spec §8/§10, M3 子项目 C). The `[keymap]` table of
//! `config.toml` maps an action id to an accelerator string:
//! - key absent  -> the action's default binding,
//! - `""`        -> explicitly unbound,
//! - value       -> overrides the default.
//!
//! Everything here is a pure string -> string transform so it is fully testable.
#![allow(dead_code)] // consumed by the keymap commands (Task 2)

use std::collections::BTreeMap;

/// Parse the `[keymap]` table; non-string values are skipped, a missing/invalid
/// file yields an empty map (all defaults).
pub fn read_keymap(text: &str) -> BTreeMap<String, String> {
    toml::from_str::<toml::Table>(text)
        .ok()
        .and_then(|t| t.get("keymap").and_then(|v| v.as_table()).cloned())
        .map(|table| {
            table
                .into_iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Return a `config.toml` body with one action's binding set:
/// `Some(accel)` writes it (`""` = unbound), `None` removes the key (back to the
/// default). Other config keys and other actions are preserved.
pub fn set_binding(text: &str, action: &str, accel: Option<&str>) -> String {
    let mut root: toml::Table = toml::from_str(text).unwrap_or_default();
    let mut keymap = root
        .remove("keymap")
        .and_then(|v| v.as_table().cloned())
        .unwrap_or_default();
    match accel {
        Some(a) => {
            keymap.insert(action.to_string(), toml::Value::String(a.to_string()));
        }
        None => {
            keymap.remove(action);
        }
    }
    if !keymap.is_empty() {
        root.insert("keymap".to_string(), toml::Value::Table(keymap));
    }
    toml::to_string_pretty(&root).unwrap_or_else(|_| text.to_string())
}

/// Drop the whole `[keymap]` table (all actions back to defaults), preserving
/// every other config key.
pub fn reset_keymap(text: &str) -> String {
    let mut root: toml::Table = toml::from_str(text).unwrap_or_default();
    root.remove("keymap");
    toml::to_string_pretty(&root).unwrap_or_else(|_| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_handles_missing_and_bad_values() {
        assert!(read_keymap("").is_empty());
        assert!(read_keymap("recovery_enabled = true\n").is_empty());
        let m = read_keymap("[keymap]\nopen = 1\nsave = \"Ctrl+S\"\n");
        assert_eq!(m.len(), 1, "non-string value skipped");
        assert_eq!(m.get("save"), Some(&"Ctrl+S".to_string()));
    }

    #[test]
    fn set_preserves_other_keys() {
        let src = "recovery_enabled = true\n\n[keymap]\nopen = \"Ctrl+O\"\n";
        let out = set_binding(src, "save", Some("Ctrl+Shift+S"));
        let map = read_keymap(&out);
        assert_eq!(map.get("open"), Some(&"Ctrl+O".to_string()));
        assert_eq!(map.get("save"), Some(&"Ctrl+Shift+S".to_string()));
        let root: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(root.get("recovery_enabled").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn set_none_removes_key() {
        let src = "[keymap]\nopen = \"Ctrl+O\"\nsave = \"Ctrl+S\"\n";
        let out = set_binding(src, "open", None);
        let map = read_keymap(&out);
        assert!(!map.contains_key("open"), "removed -> default");
        assert_eq!(map.get("save"), Some(&"Ctrl+S".to_string()));
    }

    #[test]
    fn set_none_last_key_drops_table() {
        let src = "recovery_enabled = false\n[keymap]\nopen = \"Ctrl+O\"\n";
        let out = set_binding(src, "open", None);
        assert!(read_keymap(&out).is_empty());
        assert!(out.contains("recovery_enabled = false"));
        assert!(!out.contains("[keymap]"), "empty table is dropped: {out}");
    }

    #[test]
    fn set_empty_means_unbound() {
        let out = set_binding("", "save", Some(""));
        assert_eq!(read_keymap(&out).get("save"), Some(&String::new()));
    }

    #[test]
    fn reset_only_drops_keymap() {
        let src = "recovery_enabled = true\n[keymap]\nopen = \"Ctrl+O\"\n";
        let out = reset_keymap(src);
        assert!(read_keymap(&out).is_empty());
        let root: toml::Table = toml::from_str(&out).unwrap();
        assert_eq!(root.get("recovery_enabled").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn bad_text_rebuilds() {
        let out = set_binding("bad [ toml", "open", Some("Ctrl+O"));
        assert_eq!(read_keymap(&out).get("open"), Some(&"Ctrl+O".to_string()));
    }
}
