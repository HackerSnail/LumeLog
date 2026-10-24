//! `lumelog` — M0 acceptance CLI over the pure-Rust core (no Tauri, no async).
//!
//! Subcommands (spec §3: heavy logic lives in the crate so a thin CLI can reuse
//! it and be smoke-tested):
//!
//! ```text
//! lumelog open   --file PATH [--encoding LABEL] [--line-count | --head N]
//! lumelog search --file PATH --pattern STR [--encoding LABEL] [--regex] [--ignore-case] [--max N]
//! lumelog save   --file PATH [--encoding LABEL] [--eol LF|CRLF] [--out PATH]   (new text on stdin)
//! ```
//!
//! `open` prints a single JSON summary object. `search` prints newline-delimited
//! JSON (one object per hit, then a stats object). Exit codes: `0` ok, `1` a
//! runtime/engine error, `2` a usage error (missing/unknown arguments).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use lumelog_core::doc::{save_text, DocFile, EolStyle, OverwritePolicy, SaveOptions};
use lumelog_core::encoding::EncodingLabel;
use lumelog_core::error::CoreError;
use lumelog_core::search::{search_stream, Match, Mode, SearchQuery};

/// Effectively-unlimited open size cap; the 4 GiB u32-index ceiling is still
/// enforced inside the line-index builder (`CoreError::TooLarge`).
const SIZE_LIMIT: u64 = u64::MAX;

const USAGE: &str = "usage: lumelog <open|search|save> --file PATH [options]\n\
       open   [--encoding LABEL] [--line-count | --head N]\n\
       search --pattern STR [--encoding LABEL] [--regex] [--ignore-case] [--max N]\n\
       save   [--encoding LABEL] [--eol LF|CRLF] [--out PATH]   (reads new text from stdin)\n";

/// A CLI failure. `Usage` is a bad or missing argument (exit code 2, the M0
/// acceptance contract for usage errors); `Engine` is a runtime error surfaced
/// by the core crate or the filesystem (exit code 1).
enum CliError {
    Usage(String),
    Engine(CoreError),
}

impl From<CoreError> for CliError {
    fn from(e: CoreError) -> Self {
        CliError::Engine(e)
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        CliError::Engine(e.into())
    }
}

/// A parsed command line: a flat key/value bag plus the subcommand.
struct Args {
    sub: String,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn get(&self, key: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_deref())
    }
    fn has(&self, key: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == key)
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    let args = match parse(&argv) {
        Some(a) => a,
        None => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let out = std::io::stdout();
    let mut lock = out.lock();
    let result = match args.sub.as_str() {
        "open" => cmd_open(&args, &mut lock),
        "search" => cmd_search(&args, &mut lock),
        "save" => cmd_save(&args, &mut lock),
        other => {
            eprintln!("lumelog: unknown subcommand '{other}'\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(msg)) => {
            eprintln!("lumelog: {msg}\n{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Engine(e)) => {
            eprintln!("lumelog: {e}");
            ExitCode::from(1)
        }
    }
}

/// Split `argv` into a subcommand plus `-flag [value]` pairs. A flag takes the
/// following token as its value unless that token is itself a flag (starts with
/// `-`) or is absent, in which case the flag is boolean.
fn parse(argv: &[String]) -> Option<Args> {
    let sub = argv[0].clone();
    let mut flags = Vec::new();
    let mut i = 1;
    while i < argv.len() {
        let tok = &argv[i];
        if !tok.starts_with('-') {
            return None; // stray positional argument
        }
        let key = tok.trim_start_matches('-').to_string();
        match argv.get(i + 1) {
            Some(next) if !next.starts_with('-') => {
                flags.push((key, Some(next.clone())));
                i += 2;
            }
            _ => {
                flags.push((key, None));
                i += 1;
            }
        }
    }
    Some(Args { sub, flags })
}

fn open_doc(args: &Args) -> Result<DocFile, CliError> {
    let path = args
        .get("file")
        .ok_or_else(|| CliError::Usage("missing --file".to_string()))?;
    let mut doc = DocFile::open(Path::new(path), SIZE_LIMIT)?;
    if let Some(label) = args.get("encoding") {
        let label = EncodingLabel::from_label(label)
            .ok_or_else(|| CliError::Usage(format!("unknown encoding '{label}'")))?;
        doc.set_encoding(label);
    }
    Ok(doc)
}

fn cmd_open(args: &Args, out: &mut impl Write) -> Result<(), CliError> {
    let mut doc = open_doc(args)?;
    if args.has("line-count") {
        writeln!(out, "{}", doc.line_count())?;
        return Ok(());
    }
    writeln!(
        out,
        "{{\"size\":{},\"lines\":{},\"encoding\":\"{}\",\"eol\":\"{}\"}}",
        doc.size(),
        doc.line_count(),
        doc.encoding().as_str(),
        eol_token(doc.eol_style()),
    )?;
    if let Some(head) = args.get("head") {
        let n: usize = head
            .parse()
            .map_err(|_| CliError::Usage("--head must be a number".to_string()))?;
        // Reuse the already-opened doc — a second open_doc would rebuild the
        // whole line index just to obtain a mutable handle for chunk_at.
        let chunk = doc.chunk_at(0, n)?;
        for line in chunk.text.split('\n').take(n) {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

fn cmd_search(args: &Args, out: &mut impl Write) -> Result<(), CliError> {
    let pattern_text = args
        .get("pattern")
        .ok_or_else(|| CliError::Usage("missing --pattern".to_string()))?
        .to_string();
    let mut doc = open_doc(args)?;
    let encoding = doc.encoding();
    let mode = if args.has("regex") {
        Mode::Regex
    } else {
        Mode::Literal
    };
    let case_sensitive = !args.has("ignore-case");
    let max: Option<u64> = match args.get("max") {
        Some(v) => Some(
            v.parse()
                .map_err(|_| CliError::Usage("--max must be a number".to_string()))?,
        ),
        None => None,
    };
    let q = SearchQuery {
        pattern_text,
        mode,
        encoding,
        case_sensitive,
    };

    let mut printed: u64 = 0;
    let mut overflowed = false;
    let stats = search_stream(
        &mut doc,
        &q,
        |m: Match| {
            if max.is_some_and(|cap| printed >= cap) {
                overflowed = true;
                return;
            }
            let _ = writeln!(
                out,
                "{{\"line\":{},\"byte_in_line\":{},\"len\":{},\"snippet\":\"{}\"}}",
                m.line,
                m.byte_in_line,
                m.len,
                json_escape(&m.snippet),
            );
            printed += 1;
        },
        |_| true,
    )?;
    writeln!(
        out,
        "{{\"matches\":{},\"scanned_bytes\":{},\"cancelled\":{},\"truncated\":{}}}",
        stats.matches,
        stats.scanned_bytes,
        stats.cancelled,
        overflowed,
    )?;
    Ok(())
}

fn cmd_save(args: &Args, out: &mut impl Write) -> Result<(), CliError> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let mut doc = open_doc(args)?;
    let meta = doc.disk_meta(); // before the &mut borrow of doc in save_text
    let encoding = doc.encoding();
    let eol = match args.get("eol") {
        Some(v) => match v.to_ascii_lowercase().as_str() {
            "lf" => EolStyle::Lf,
            "crlf" => EolStyle::Crlf,
            other => {
                return Err(CliError::Usage(format!("unknown --eol '{other}'")));
            }
        },
        None => doc.eol_style(),
    };
    let path = args.get("out").map(PathBuf::from);
    let opts = SaveOptions {
        path,
        encoding,
        eol,
        overwrite_policy: OverwritePolicy::SafetyConfirm,
    };
    let report = save_text(&mut doc, &text, &opts, meta)?;
    writeln!(
        out,
        "{{\"saved\":true,\"replace_count\":{},\"conflict\":{}}}",
        report.replace_count, report.conflict
    )?;
    Ok(())
}

fn eol_token(eol: EolStyle) -> &'static str {
    match eol {
        EolStyle::Lf => "lf",
        EolStyle::Crlf => "crlf",
        EolStyle::Mixed => "mixed",
    }
}

/// Escape a Rust string into a JSON string body (no surrounding quotes). Log
/// snippets can carry quotes, backslashes and control bytes.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
