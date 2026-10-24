//! lumelog-can decode CLI (spec §7.5 / §12 M2 engine acceptance).
//!
//! Hand-rolled `std::env::args` parsing — no clap, no serde (JSON/CSV via
//! `format!`). Two subcommands:
//!
//!   lumelog-can dbc    --file PATH [--lenient]
//!   lumelog-can decode --log PATH --format candump|asc|regex
//!                      [--regex STR] [--dbc PATH] [--limit N] [--out jsonl|csv]
//!
//! Exit codes: 2 = usage error, 1 = file/parse error.

use std::process::ExitCode;

use lumelog_can::dbc::{parse, parse_lenient, Database, Message};
use lumelog_can::decode::decode_signals;
use lumelog_can::frame::Frame;
use lumelog_can::source::{AscSource, CandumpSource, FrameSource, Line, RegexSource};

const USAGE: &str = "usage:
  lumelog-can dbc    --file PATH [--lenient]
  lumelog-can decode --log PATH --format candump|asc|regex [--regex STR] [--dbc PATH] [--limit N] [--out jsonl|csv]

exit codes: 2 = usage error, 1 = file/parse error";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run(&args))
}

fn run(args: &[String]) -> u8 {
    let Some(sub) = args.first() else {
        eprintln!("{USAGE}");
        return 2;
    };
    match sub.as_str() {
        "dbc" => cmd_dbc(&args[1..]),
        "decode" => cmd_decode(&args[1..]),
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            0
        }
        other => {
            eprintln!("unknown subcommand: {other}\n{USAGE}");
            2
        }
    }
}

/// Value of `--flag NEXT`, if `--flag` is present.
fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).map(String::as_str))
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn read_lines(path: &str) -> Option<Vec<Line>> {
    match std::fs::read_to_string(path) {
        Ok(txt) => Some(
            txt.lines()
                .enumerate()
                .map(|(i, l)| ((i + 1) as u64, l.to_string()))
                .collect(),
        ),
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            None
        }
    }
}

fn load_db(path: &str) -> Option<Database> {
    let src = std::fs::read_to_string(path).map_err(|e| eprintln!("cannot read {path}: {e}")).ok()?;
    Some(parse_lenient(&src).database)
}

fn cmd_dbc(args: &[String]) -> u8 {
    let Some(path) = flag(args, "--file") else {
        eprintln!("dbc: --file PATH is required\n{USAGE}");
        return 2;
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        eprintln!("dbc: cannot read {path}");
        return 1;
    };

    let (db, errors) = if has(args, "--lenient") {
        let outcome = parse_lenient(&text);
        let n = outcome.errors.len();
        (outcome.database, n)
    } else {
        match parse(&text) {
            Ok(db) => (db, 0usize),
            Err(e) => {
                eprintln!("dbc: {e}");
                return 1;
            }
        }
    };

    let signals: usize = db.messages.iter().map(|m| m.signals.len()).sum();
    println!("{{\"messages\":{},\"signals\":{},\"errors\":{}}}", db.messages.len(), signals, errors);
    0
}

fn cmd_decode(args: &[String]) -> u8 {
    let Some(log) = flag(args, "--log") else {
        eprintln!("decode: --log PATH is required\n{USAGE}");
        return 2;
    };
    let Some(format) = flag(args, "--format") else {
        eprintln!("decode: --format candump|asc|regex is required\n{USAGE}");
        return 2;
    };
    if let Some(bad) = flag(args, "--out").filter(|o| !matches!(*o, "jsonl" | "csv")) {
        eprintln!("decode: --out must be jsonl or csv (got {bad})");
        return 2;
    }
    let out = flag(args, "--out");
    let limit = match flag(args, "--limit") {
        Some(s) => match s.parse::<usize>() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("decode: --limit must be a non-negative integer (got {s})");
                return 2;
            }
        },
        None => 20,
    };
    let dbc: Option<Database> = flag(args, "--dbc").map(|p| {
        load_db(p).unwrap_or_else(|| {
            eprintln!("decode: cannot read dbc {p}");
            std::process::exit(1);
        })
    });

    let mut lines = match read_lines(log) {
        Some(l) => l,
        None => return 1,
    };
    // Sort not needed; sources consume in order. Build the selected source.
    let mut src: Box<dyn FrameSource> = match format {
        "candump" => Box::new(CandumpSource::new(lines.drain(..))),
        "asc" => Box::new(AscSource::new(lines.drain(..))),
        "regex" => {
            let Some(pat) = flag(args, "--regex") else {
                eprintln!("decode: --format regex requires --regex STR");
                return 2;
            };
            match RegexSource::new(lines.drain(..), pat) {
                Ok(s) => Box::new(s),
                Err(e) => {
                    eprintln!("decode: {e}");
                    return 2;
                }
            }
        }
        other => {
            eprintln!("decode: --format must be candump|asc|regex (got {other})\n{USAGE}");
            return 2;
        }
    };

    if out == Some("csv") {
        println!("line,ts,channel,id,dlc,data");
    }

    let print_all = matches!(out, Some("jsonl") | Some("csv"));
    let mut printed = 0usize;
    while let Some(f) = src.next_frame() {
        let want = print_all || printed < limit;
        if !want {
            continue; // still drain to reach the end-of-stream stats
        }
        printed += 1;
        match out {
            Some("csv") => println!("{}", frame_csv(&f)),
            _ => println!("{}", frame_json(&f, dbc_msg(dbc.as_ref(), &f))),
        }
    }

    let st = src.stats();
    match out {
        Some("csv") => eprintln!("# decoded={} skipped={}", st.decoded, st.skipped),
        _ => println!("{{\"decoded\":{},\"skipped\":{}}}", st.decoded, st.skipped),
    }
    0
}

fn dbc_msg<'a>(db: Option<&'a Database>, f: &Frame) -> Option<&'a Message> {
    db?.messages.iter().find(|m| m.id == f.can_id)
}

fn data_hex(f: &Frame) -> String {
    f.data[..f.dlc as usize].iter().map(|b| format!("{b:02X}")).collect()
}

fn frame_csv(f: &Frame) -> String {
    let ts = f.timestamp.map(|t| t.to_string()).unwrap_or_default();
    format!("{},{},{},0x{:X},{},{}", f.line_number, ts, f.channel, f.can_id, f.dlc, data_hex(f))
}

fn frame_json(f: &Frame, msg: Option<&Message>) -> String {
    let ts = f.timestamp.map(|t| t.to_string()).unwrap_or_else(|| "null".to_string());
    let mut s = format!(
        "{{\"line\":{},\"ts\":{ts},\"channel\":{},\"id\":\"0x{:X}\",\"dlc\":{},\"data\":\"{}\"",
        f.line_number, f.channel, f.can_id, f.dlc, data_hex(f)
    );
    if let Some(m) = msg {
        let sigs: Vec<String> = decode_signals(m, &f.data[..f.dlc as usize])
            .iter()
            .map(|sv| {
                format!(
                    "{{\"name\":\"{}\",\"raw\":{},\"physical\":{},\"unit\":\"{}\"}}",
                    json_escape(&sv.name),
                    sv.raw,
                    sv.physical,
                    json_escape(&sv.unit)
                )
            })
            .collect();
        s.push_str(&format!(",\"message_name\":\"{}\"", json_escape(&m.name)));
        s.push_str(&format!(",\"signals\":[{}]", sigs.join(",")));
    }
    s.push('}');
    s
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
