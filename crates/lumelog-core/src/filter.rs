//! Multi-condition line filtering (spec §6.4, M3). A pure AND-of-includes /
//! NOT-of-excludes predicate over **decoded line text**, returning the include
//! matches as **UTF-16 (cu16)** highlight ranges so the front end can slice the
//! rendered line directly with `String.slice` — no `char_map`/`DocFile` needed
//! (byte→cu16 is computed from the line text itself, which is exactly the text
//! `doc_lines_at`/`loadWindow` hand back, so the offsets line up).
//!
//! Case: a `case_sensitive=false` query builds a case-insensitive regex; a
//! literal is `regex::escape`d first so metacharacters match literally.

use chrono::{DateTime, NaiveDateTime};
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

/// Built-in timestamp formats for the timeline axis (M3 spec §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeFormat {
    Iso,
    EpochSec,
    EpochMillis,
}

/// A timeline constraint: `extract` must be a regex with a named group `ts`;
/// each line's captured timestamp text is parsed by `format` into milliseconds
/// and kept when within `[from, to]`. `keep_when_no_ts` decides whether lines
/// whose `ts` is absent/unparsable still survive the time axis.
#[derive(Debug, Clone)]
pub struct TimeSpec {
    pub extract: String,
    pub format: TimeFormat,
    pub from: Option<String>,
    pub to: Option<String>,
    pub keep_when_no_ts: bool,
}

/// Parse a timestamp string to epoch milliseconds under the chosen format.
/// `Iso` accepts RFC3339 (with offset) or `YYYY-MM-DD[T ]HH:MM:SS[.frac]` (UTC).
fn parse_time_ms(s: &str, format: TimeFormat) -> Option<f64> {
    let t = s.trim();
    match format {
        TimeFormat::EpochSec => t.parse::<f64>().ok().map(|v| v * 1000.0),
        TimeFormat::EpochMillis => t.parse::<f64>().ok(),
        TimeFormat::Iso => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
                return Some(dt.timestamp_millis() as f64);
            }
            for pat in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
                if let Ok(naive) = NaiveDateTime::parse_from_str(t, pat) {
                    return Some(naive.and_utc().timestamp_millis() as f64);
                }
            }
            None
        }
    }
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

#[derive(Debug)]
struct CompiledTime {
    extract: Regex,
    format: TimeFormat,
    from_ms: Option<f64>,
    to_ms: Option<f64>,
    keep_when_no_ts: bool,
}

/// A compiled multi-condition AND filter with optional timeline + line-range
/// axes. Build once, call [`LineFilter::keep`] per line.
#[derive(Debug)]
pub struct LineFilter {
    conds: Vec<CompiledCond>,
    time: Option<CompiledTime>,
    line_range: Option<(u64, u64)>,
}

impl LineFilter {
    /// Compile every condition into a regex (literals escaped). Rejects an empty
    /// condition text and invalid regex syntax with [`CoreError::InvalidPattern`].
    pub fn build(
        specs: Vec<CondSpec>,
        case_sensitive: bool,
        time: Option<TimeSpec>,
        line_range: Option<(u64, u64)>,
    ) -> Result<LineFilter, CoreError> {
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
        let compiled_time = match time {
            None => None,
            Some(t) => {
                let extract = Regex::new(&t.extract)
                    .map_err(|e| CoreError::InvalidPattern(format!("invalid time extract regex: {e}")))?;
                if !extract.capture_names().flatten().any(|n| n == "ts") {
                    return Err(CoreError::InvalidPattern(
                        "time extract regex must define a named group `ts`".to_string(),
                    ));
                }
                let from_ms = t.from.as_ref().map(|s| {
                    parse_time_ms(s, t.format)
                        .ok_or_else(|| CoreError::InvalidPattern(format!("invalid 'from' timestamp: {s}")))
                }).transpose()?;
                let to_ms = t.to.as_ref().map(|s| {
                    parse_time_ms(s, t.format)
                        .ok_or_else(|| CoreError::InvalidPattern(format!("invalid 'to' timestamp: {s}")))
                }).transpose()?;
                Some(CompiledTime {
                    extract,
                    format: t.format,
                    from_ms,
                    to_ms,
                    keep_when_no_ts: t.keep_when_no_ts,
                })
            }
        };
        Ok(LineFilter { conds, time: compiled_time, line_range })
    }

    /// Whether ANY axis was supplied (text conditions, a timeline, or a line
    /// range). The caller treats `false` as "do not scan" — an all-empty query
    /// would otherwise keep every line.
    pub fn has_conditions(&self) -> bool {
        !self.conds.is_empty() || self.time.is_some() || self.line_range.is_some()
    }

    /// Keep-decision for one decoded line, given its 0-based `line_no`. Returns
    /// `Some(ranges)` when the line survives all active axes — text AND,
    /// optional line-range, optional timeline — with `ranges` the union of the
    /// include matches (empty when only excludes / no include); `None` otherwise.
    pub fn keep(&self, line_no: u64, line_text: &str) -> Option<Vec<Cu16Range>> {
        // Line-range axis (1-based inclusive); the degrade path when no extract regex.
        if let Some((from, to)) = self.line_range {
            let one = line_no + 1;
            if one < from || one > to {
                return None;
            }
        }
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
        // Timeline axis: extract `ts`, parse, compare to the window.
        if let Some(t) = &self.time {
            let ms = t
                .extract
                .captures(line_text)
                .and_then(|caps| caps.name("ts"))
                .and_then(|m| parse_time_ms(m.as_str(), t.format));
            match ms {
                Some(v) => {
                    if t.from_ms.map(|f| v < f).unwrap_or(false) {
                        return None;
                    }
                    if t.to_ms.map(|to| v > to).unwrap_or(false) {
                        return None;
                    }
                }
                None => {
                    if !t.keep_when_no_ts {
                        return None;
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
    fn b(specs: Vec<CondSpec>, case: bool) -> LineFilter {
        LineFilter::build(specs, case, None, None).unwrap()
    }
    fn iso(from: Option<&str>, to: Option<&str>, keep: bool) -> TimeSpec {
        TimeSpec {
            extract: r"(?<ts>\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2})".to_string(),
            format: TimeFormat::Iso,
            from: from.map(|s| s.to_string()),
            to: to.map(|s| s.to_string()),
            keep_when_no_ts: keep,
        }
    }

    #[test]
    fn literal_include_keeps_matching_lines() {
        let lf = b(vec![inc(CondKind::Literal, "错误")], true);
        assert_eq!(lf.keep(0, "系统 错误 记录"), Some(vec![Cu16Range { start: 3, len: 2 }]));
        assert_eq!(lf.keep(0, "正常 心跳"), None);
    }

    #[test]
    fn and_semantics_all_includes_required() {
        let lf = b(vec![inc(CondKind::Literal, "a"), inc(CondKind::Literal, "b")], true);
        assert_eq!(lf.keep(0, "only a here"), None, "missing b -> excluded");
        assert_eq!(lf.keep(0, "b then a").expect("both present").len(), 2);
    }

    #[test]
    fn exclude_vetoes() {
        let lf = b(vec![inc(CondKind::Literal, "keep"), exc("drop")], true);
        assert_eq!(lf.keep(0, "keep drop"), None);
        assert!(lf.keep(0, "keep ok").is_some());
    }

    #[test]
    fn empty_include_only_exclude_vacuous_true() {
        let lf = b(vec![exc("x")], true);
        assert_eq!(lf.keep(0, "abc"), Some(vec![]));
        assert_eq!(lf.keep(0, "x ray"), None);
    }

    #[test]
    fn regex_case_insensitive() {
        let ci = b(vec![inc(CondKind::Regex, "Error")], false);
        assert!(ci.keep(0, "an error here").is_some());
        let cs = b(vec![inc(CondKind::Regex, "Error")], true);
        assert_eq!(cs.keep(0, "an error here"), None, "case-sensitive misses lowercase");
        assert!(cs.keep(0, "Error").is_some());
    }

    #[test]
    fn literal_metachar_is_literal() {
        let lf = b(vec![inc(CondKind::Literal, "(a")], true);
        assert!(lf.keep(0, "has (a inside").is_some());
        assert_eq!(lf.keep(0, "regex a would match if not escaped"), None);
    }

    #[test]
    fn utf16_offsets_for_multibyte() {
        let lf = b(vec![inc(CondKind::Literal, "\u{1F600}")], true);
        assert_eq!(lf.keep(0, "错误 \u{1F600}x").unwrap(), vec![Cu16Range { start: 3, len: 2 }]);
    }

    #[test]
    fn empty_pattern_rejected() {
        let err = LineFilter::build(vec![inc(CondKind::Literal, "")], true, None, None).unwrap_err();
        assert!(matches!(err, CoreError::InvalidPattern(_)), "{err:?}");
    }

    #[test]
    fn invalid_regex_rejected() {
        let err = LineFilter::build(vec![inc(CondKind::Regex, "(")], true, None, None).unwrap_err();
        assert!(matches!(err, CoreError::InvalidPattern(_)), "{err:?}");
    }

    #[test]
    fn has_conditions_false_when_empty() {
        let lf = LineFilter::build(vec![], true, None, None).unwrap();
        assert!(!lf.has_conditions(), "all-empty filter must be 'do not scan'");
    }

    // ---- M3 increment 2: timeline + line-range axes ----

    #[test]
    fn line_range_keeps_only_within() {
        let lf = LineFilter::build(vec![inc(CondKind::Literal, "x")], true, None, Some((2, 3))).unwrap();
        assert!(lf.keep(0, "x").is_none(), "line 1 out of [2,3]");
        assert!(lf.keep(1, "x").is_some(), "line 2 in");
        assert!(lf.keep(2, "x").is_some(), "line 3 in");
        assert!(lf.keep(3, "x").is_none(), "line 4 out");
    }

    #[test]
    fn time_iso_window() {
        let lf = LineFilter::build(vec![], true, Some(iso(Some("2026-01-01 00:00:00"), Some("2026-01-02 00:00:00"), false)), None).unwrap();
        assert!(lf.keep(0, "2026-01-01 06:00:00 ok").is_some());
        assert!(lf.keep(0, "2025-12-31 23:00:00").is_none());
    }

    #[test]
    fn time_epoch_seconds() {
        let t = TimeSpec {
            extract: r"(?<ts>\d+)".to_string(),
            format: TimeFormat::EpochSec,
            from: Some("1000".to_string()),
            to: None,
            keep_when_no_ts: false,
        };
        let lf = LineFilter::build(vec![], true, Some(t), None).unwrap();
        assert!(lf.keep(0, "t=1500 x").is_some(), "1.5e6 >= 1.0e6 ms");
        assert!(lf.keep(0, "t=500 x").is_none(), "5e5 < 1.0e6 ms");
    }

    #[test]
    fn keep_when_no_ts_flags() {
        let keep = LineFilter::build(vec![], true, Some(iso(None, None, true)), None).unwrap();
        let drop = LineFilter::build(vec![], true, Some(iso(None, None, false)), None).unwrap();
        assert_eq!(keep.keep(0, "no timestamp here"), Some(vec![]));
        assert_eq!(drop.keep(0, "no timestamp here"), None);
    }

    #[test]
    fn extract_missing_ts_group_errors() {
        let t = TimeSpec {
            extract: "(a)".to_string(),
            format: TimeFormat::Iso,
            from: None,
            to: None,
            keep_when_no_ts: false,
        };
        let err = LineFilter::build(vec![], true, Some(t), None).unwrap_err();
        assert!(matches!(err, CoreError::InvalidPattern(_)), "{err:?}");
    }

    #[test]
    fn iso_offset_and_fraction_equal() {
        let base = parse_time_ms("2026-01-01T00:00:00Z", TimeFormat::Iso);
        let with_offset = parse_time_ms("2026-01-01T08:00:00+08:00", TimeFormat::Iso);
        assert_eq!(base, with_offset, "+08:00 at 08:00 == 00:00Z instant");
        let frac = parse_time_ms("2026-01-01T00:00:00.500Z", TimeFormat::Iso);
        assert_eq!(frac, base.map(|v| v + 500.0), ".500 -> +500 ms");
        assert_eq!(
            parse_time_ms("2026-01-01 00:00:00", TimeFormat::Iso),
            base,
            "space form == T form, no offset treated as UTC"
        );
    }
}
