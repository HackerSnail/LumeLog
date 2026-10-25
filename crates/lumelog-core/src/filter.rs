//! Multi-condition line filtering (spec §6.4, M3). A pure AND-of-includes /
//! NOT-of-excludes predicate over **decoded line text**, returning the include
//! matches as **UTF-16 (cu16)** highlight ranges so the front end can slice the
//! rendered line directly with `String.slice` — no `char_map`/`DocFile` needed
//! (byte→cu16 is computed from the line text itself, which is exactly the text
//! `doc_lines_at`/`loadWindow` hand back, so the offsets line up).
//!
//! Case: a `case_sensitive=false` query builds a case-insensitive regex; a
//! literal is `regex::escape`d first so metacharacters match literally.

use regex::{Regex, RegexBuilder};

use crate::error::CoreError;

/// Whether a condition keeps lines (`Include`) or vetoes them (`Exclude`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondMode {
    Include,
    Exclude,
}

/// How a condition's text is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CondKind {
    Literal,
    Regex,
}

/// One un-compiled filter condition as it arrives from the IPC layer.
#[derive(Debug, Clone)]
pub struct CondSpec {
    pub mode: CondMode,
    pub kind: CondKind,
    pub text: String,
}

/// A highlight span in UTF-16 code units within a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cu16Range {
    pub start: u32,
    pub len: u32,
}

/// Count UTF-16 code units in `text[..byte_idx]` (byte offset of a regex hit).
fn cu16_before(text: &str, byte_idx: usize) -> u32 {
    text[..byte_idx].chars().map(char::len_utf16).sum::<usize>() as u32
}

/// Count UTF-16 code units in `text[from..to]`.
fn cu16_len(text: &str, from: usize, to: usize) -> u32 {
    text[from..to].chars().map(char::len_utf16).sum::<usize>() as u32
}

#[derive(Debug)]
struct CompiledCond {
    mode: CondMode,
    re: Regex,
}

/// A compiled multi-condition AND filter. Build once, call [`LineFilter::keep`]
/// per line.
#[derive(Debug)]
pub struct LineFilter {
    conds: Vec<CompiledCond>,
}

impl LineFilter {
    /// Compile every condition into a regex (literals escaped). Rejects an empty
    /// condition text and invalid regex syntax with [`CoreError::InvalidPattern`].
    pub fn build(specs: Vec<CondSpec>, case_sensitive: bool) -> Result<LineFilter, CoreError> {
        let mut conds = Vec::with_capacity(specs.len());
        for s in specs {
            if s.text.is_empty() {
                return Err(CoreError::InvalidPattern("empty filter condition".to_string()));
            }
            let pattern = match s.kind {
                // Escape so a literal's regex metacharacters (and arbitrary byte
                // values) match as text, mirroring `search::build_regex`.
                CondKind::Literal => regex::escape(&s.text),
                CondKind::Regex => s.text.clone(),
            };
            let re = RegexBuilder::new(&pattern)
                .case_insensitive(!case_sensitive)
                .build()
                .map_err(|e| CoreError::InvalidPattern(e.to_string()))?;
            conds.push(CompiledCond { mode: s.mode, re });
        }
        Ok(LineFilter { conds })
    }

    /// Whether any condition was supplied. The caller must treat `false` as
    /// "do not scan" (an empty filter would otherwise keep every line).
    pub fn has_conditions(&self) -> bool {
        !self.conds.is_empty()
    }

    /// Keep-decision for one decoded line. `Some(ranges)` = kept (ranges are the
    /// union of the include matches, ascending & de-duplicated; empty when only
    /// excludes are present — the line is kept but not highlighted); `None` =
    /// excluded (some include did not match, or some exclude did).
    pub fn keep(&self, line_text: &str) -> Option<Vec<Cu16Range>> {
        let mut ranges: Vec<Cu16Range> = Vec::new();
        for c in &self.conds {
            match c.mode {
                CondMode::Include => {
                    let mut matched = false;
                    for m in c.re.find_iter(line_text) {
                        matched = true;
                        ranges.push(Cu16Range {
                            start: cu16_before(line_text, m.start()),
                            len: cu16_len(line_text, m.start(), m.end()),
                        });
                    }
                    if !matched {
                        return None; // AND-of-includes fails
                    }
                }
                CondMode::Exclude => {
                    if c.re.is_match(line_text) {
                        return None; // any exclude vetoes
                    }
                }
            }
        }
        if ranges.len() > 1 {
            ranges.sort_by_key(|r| (r.start, r.len));
            ranges.dedup();
        }
        Some(ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inc(kind: CondKind, text: &str) -> CondSpec {
        CondSpec { mode: CondMode::Include, kind, text: text.to_string() }
    }
    fn exc(text: &str) -> CondSpec {
        CondSpec { mode: CondMode::Exclude, kind: CondKind::Literal, text: text.to_string() }
    }

    #[test]
    fn literal_include_keeps_matching_lines() {
        let lf = LineFilter::build(vec![inc(CondKind::Literal, "错误")], true).unwrap();
        assert_eq!(lf.keep("系统 错误 记录"), Some(vec![Cu16Range { start: 3, len: 2 }]));
        assert_eq!(lf.keep("正常 心跳"), None);
    }

    #[test]
    fn and_semantics_all_includes_required() {
        let lf = LineFilter::build(
            vec![inc(CondKind::Literal, "a"), inc(CondKind::Literal, "b")],
            true,
        )
        .unwrap();
        assert_eq!(lf.keep("only a here"), None, "missing b -> excluded");
        let kept = lf.keep("b then a").expect("both present");
        assert_eq!(kept.len(), 2, "one range per include match");
    }

    #[test]
    fn exclude_vetoes() {
        let lf = LineFilter::build(
            vec![inc(CondKind::Literal, "keep"), exc("drop")],
            true,
        )
        .unwrap();
        assert_eq!(lf.keep("keep drop"), None);
        assert!(lf.keep("keep ok").is_some());
    }

    #[test]
    fn empty_include_only_exclude_vacuous_true() {
        let lf = LineFilter::build(vec![exc("x")], true).unwrap();
        // kept but no include ranges (exclude-only)
        assert_eq!(lf.keep("abc"), Some(vec![]));
        assert_eq!(lf.keep("x ray"), None);
    }

    #[test]
    fn regex_case_insensitive() {
        let ci = LineFilter::build(vec![inc(CondKind::Regex, "Error")], false).unwrap();
        assert!(ci.keep("an error here").is_some());
        assert!(ci.keep("ERROR").is_some());
        let cs = LineFilter::build(vec![inc(CondKind::Regex, "Error")], true).unwrap();
        assert_eq!(cs.keep("an error here"), None, "case-sensitive misses lowercase");
        assert!(cs.keep("Error").is_some());
    }

    #[test]
    fn literal_metachar_is_literal() {
        let lf = LineFilter::build(vec![inc(CondKind::Literal, "(a")], true).unwrap();
        assert!(lf.keep("has (a inside").is_some());
        assert_eq!(lf.keep("regex a would match if not escaped"), None);
    }

    #[test]
    fn utf16_offsets_for_multibyte() {
        // "错误 <emoji>x": emoji is U+1F600 (UTF-16 length 2). Preceding cu16:
        // 错(1)+误(1)+space(1) = 3.
        let lf = LineFilter::build(vec![inc(CondKind::Literal, "\u{1F600}")], true).unwrap();
        let kept = lf.keep("错误 \u{1F600}x").expect("emoji present");
        assert_eq!(kept, vec![Cu16Range { start: 3, len: 2 }]);
    }

    #[test]
    fn empty_pattern_rejected() {
        let err = LineFilter::build(vec![inc(CondKind::Literal, "")], true).unwrap_err();
        assert!(matches!(err, CoreError::InvalidPattern(_)), "{err:?}");
    }

    #[test]
    fn invalid_regex_rejected() {
        let err = LineFilter::build(vec![inc(CondKind::Regex, "(")], true).unwrap_err();
        assert!(matches!(err, CoreError::InvalidPattern(_)), "{err:?}");
    }

    #[test]
    fn has_conditions_false_when_empty() {
        let lf = LineFilter::build(vec![], true).unwrap();
        assert!(!lf.has_conditions(), "empty filter must be treated as 'do not scan'");
    }
}
