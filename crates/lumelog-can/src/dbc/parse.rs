//! DBC subset parser (spec §7.1). Line-oriented: a strict `parse` aborts on the
//! first malformed construct; `parse_lenient` records it (with 1-based line +
//! context) and keeps going ("跳过坏行继续解析"). Only the grammar subset the
//! spec names is modelled; comments (`CM_`), attributes (`BA_*`) and value tables
//! (`VAL_TABLE_`) are recognised and skipped without gaining semantics.

use super::{ByteOrder, Database, Message, Mux, Signal};
use crate::error::CanError;
use regex::{Captures, Regex};

/// Result of a lenient parse: the best-effort database plus every skipped line.
#[derive(Debug)]
pub struct ParseOutcome {
    pub database: Database,
    pub errors: Vec<CanError>,
}

/// Strict parse: aborts at the first bad construct.
pub fn parse(src: &str) -> Result<Database, CanError> {
    parse_impl(src, false).map(|o| o.database)
}

/// Lenient parse: never fails; malformed lines land in `errors`.
pub fn parse_lenient(src: &str) -> ParseOutcome {
    parse_impl(src, true).expect("lenient parse collects errors instead of failing")
}

fn syntax(line: usize, ctx: &str, kind: &str) -> CanError {
    CanError::DbcSyntax { line, context: ctx.to_string(), kind: kind.to_string() }
}

/// In lenient mode push the error and continue; in strict mode return it.
fn note(errors: &mut Vec<CanError>, lenient: bool, e: CanError) -> Result<(), CanError> {
    if lenient {
        errors.push(e);
        Ok(())
    } else {
        Err(e)
    }
}

/// A queued `VAL_` application: (masked message id, signal name, enum table).
type ValEntry = (u32, String, Vec<(i64, String)>);

fn parse_impl(src: &str, lenient: bool) -> Result<ParseOutcome, CanError> {
    // Each regex is compiled once per file; DBC import is a one-shot operation.
    let bo = Regex::new(r"^BO_\s+(\d+)\s+(\w+)\s*:\s*(\d+)\s*(\w*)").unwrap();
    let sg = Regex::new(concat!(
        r"^SG_\s+(\w+)\s*:\s+",
        r"(?:(M|m\d+)\s+)?",
        r"(\d+)\|(\d+)@([01])([+-])\s+",
        r"\(([^,]+),([^)]+)\)\s+",
        r"\[([^|]*)\|([^\]]*)\]\s+",
        r#""([^"]*)"\s*"#,
        r"(.*)$"
    ))
    .unwrap();
    let val = Regex::new(r"^VAL_\s+(\d+)\s+(\w+)\s+(.*);").unwrap();
    let version = Regex::new(r#"^VERSION\s+"([^"]*)""#).unwrap();
    let pair = Regex::new(r#"(-?\d+)\s+"([^"]*)""#).unwrap();

    let mut db = Database::default();
    let mut errors: Vec<CanError> = Vec::new();
    let mut current: Option<usize> = None;
    // VAL_ lines reference (masked id, signal name); applied after the scan.
    let mut val_queue: Vec<ValEntry> = Vec::new();

    for (idx, raw) in src.lines().enumerate() {
        let lineno = idx + 1;
        let t = raw.trim();
        if t.is_empty() || t.starts_with("//") {
            continue;
        }

        if t.starts_with("VERSION") {
            if let Some(c) = version.captures(t) {
                db.version = c[1].to_string();
            }
        } else if t.starts_with("NS_")
            || t.starts_with("BS_")
            || t.starts_with("VAL_TABLE_")
            || t.starts_with("CM_")
            || t.starts_with("BA_")
        {
            // namespace / baudrate / value-table / comment / attribute: skipped
            // (spec §7.1 — semantics not implemented, must not error).
        } else if t.starts_with("BU_") {
            let rest = t.trim_start_matches("BU_").trim_start_matches(':');
            db.nodes = rest.split_whitespace().map(|s| s.to_string()).collect();
        } else if t.starts_with("BO_") {
            match bo.captures(t) {
                Some(c) => match build_message(lineno, t, &c) {
                    Ok(m) => {
                        db.messages.push(m);
                        current = Some(db.messages.len() - 1);
                    }
                    Err(e) => note(&mut errors, lenient, e)?,
                },
                None => note(&mut errors, lenient, syntax(lineno, t, "bad BO_"))?,
            }
        } else if t.starts_with("SG_") {
            match sg.captures(t) {
                Some(c) => match build_signal(lineno, t, &c) {
                    Ok(sig) => match current {
                        Some(mi) => db.messages[mi].signals.push(sig),
                        None => {
                            note(&mut errors, lenient, syntax(lineno, t, "SG_ outside BO_"))?
                        }
                    },
                    Err(e) => note(&mut errors, lenient, e)?,
                },
                None => note(&mut errors, lenient, syntax(lineno, t, "bad SG_"))?,
            }
        } else if t.starts_with("VAL_") {
            match val.captures(t) {
                Some(c) => {
                    let id = c[1].parse::<u32>().unwrap_or(0) & 0x1FFF_FFFF;
                    let name = c[2].to_string();
                    let table = pair
                        .captures_iter(&c[3])
                        .map(|p| (p[1].parse::<i64>().unwrap_or(0), p[2].to_string()))
                        .collect();
                    val_queue.push((id, name, table));
                }
                None => note(&mut errors, lenient, syntax(lineno, t, "bad VAL_"))?,
            }
        }
        // Any other unrecognised line (e.g. `[NODE]` inside NS_) is ignored: the
        // subset does not model it and it is not a malformed construct.
    }

    // Attach enum value tables to their signals now that all messages exist.
    for (id, name, table) in val_queue {
        if let Some(m) = db.messages.iter_mut().find(|m| m.id == id) {
            if let Some(s) = m.signals.iter_mut().find(|s| s.name == name) {
                s.value_table = table;
            }
        }
    }

    Ok(ParseOutcome { database: db, errors })
}

fn build_message(lineno: usize, ctx: &str, c: &Captures) -> Result<Message, CanError> {
    let raw_id: u32 = c[1].parse().map_err(|_| syntax(lineno, ctx, "bad BO_ id"))?;
    let dlc: u8 = c[3].parse().map_err(|_| syntax(lineno, ctx, "bad BO_ dlc"))?;
    Ok(Message {
        id: raw_id & 0x1FFF_FFFF,
        name: c[2].to_string(),
        dlc,
        transmitter: c[4].to_string(),
        is_extended: raw_id & 0x8000_0000 != 0,
        signals: Vec::new(),
    })
}

fn build_signal(lineno: usize, ctx: &str, c: &Captures) -> Result<Signal, CanError> {
    let mux = match c.get(2).map(|m| m.as_str()) {
        None => Mux::NonMuxed,
        Some("M") => Mux::Selector,
        Some(s) => {
            let n: u8 = s[1..].parse().map_err(|_| syntax(lineno, ctx, "bad mux index"))?;
            Mux::Indexed(n)
        }
    };
    let start_bit: u32 = c[3].parse().map_err(|_| syntax(lineno, ctx, "bad start_bit"))?;
    let length: u32 = c[4].parse().map_err(|_| syntax(lineno, ctx, "bad length"))?;
    let byte_order = if &c[5] == "1" { ByteOrder::LittleEndian } else { ByteOrder::BigEndian };
    let is_signed = &c[6] == "-";
    let factor: f64 = c[7].trim().parse().map_err(|_| syntax(lineno, ctx, "bad factor"))?;
    let offset: f64 = c[8].trim().parse().map_err(|_| syntax(lineno, ctx, "bad offset"))?;
    let minimum: f64 = c[9].trim().parse().unwrap_or(0.0);
    let maximum: f64 = c[10].trim().parse().unwrap_or(0.0);
    let receivers = c[12].split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).map(String::from).collect();
    Ok(Signal {
        name: c[1].to_string(),
        start_bit,
        length,
        byte_order,
        is_signed,
        factor,
        offset,
        minimum,
        maximum,
        unit: c[11].to_string(),
        receivers,
        value_table: Vec::new(),
        mux,
    })
}
