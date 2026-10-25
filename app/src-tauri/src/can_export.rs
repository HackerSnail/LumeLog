//! CAN view-filter predicate + (later) export text builders — spec §7.5 / §9,
//! M3 increment 3. `frame_matches` is a faithful port of the front-end
//! `canfilter.ts` predicate and operates on the SAME `FrameJson` shape the panel
//! renders, so backend export can never drift from what the user sees.

use crate::dto::{CanFilterJson, FrameJson, SignalOp};

/// True iff `fj` satisfies every active axis of `f` (AND), matching
/// `canfilter.ts`'s `applyCanFilter` exactly — including `Object.is` float
/// equality for `==` (bit-equal: `NaN==NaN` is true, `+0 != -0`).
#[allow(dead_code)] // consumed by `export_can` (Task 4)
pub fn frame_matches(fj: &FrameJson, f: &CanFilterJson) -> bool {
    // ID closed range; a reversed range matches nothing.
    if let Some(from) = f.id_from {
        if fj.can_id < from {
            return false;
        }
    }
    if let Some(to) = f.id_to {
        if fj.can_id > to {
            return false;
        }
    }
    if let (Some(from), Some(to)) = (f.id_from, f.id_to) {
        if from > to {
            return false;
        }
    }
    // Message name exact (frames without a msg name are dropped when the axis is set).
    if let Some(name) = &f.msg_name {
        if fj.msg_name.as_deref() != Some(name.as_str()) {
            return false;
        }
    }
    // Signal condition: active only when name + op + value are all present.
    if let (Some(sig), Some(op), Some(val)) = (&f.signal_name, f.signal_op, f.signal_value) {
        let physical = match fj.signals.iter().find(|s| &s.name == sig) {
            Some(s) => s.physical,
            None => return false, // no such signal -> excluded (Object.is-safe)
        };
        let ok = match op {
            SignalOp::Gt => physical > val,
            SignalOp::Lt => physical < val,
            SignalOp::Ge => physical >= val,
            SignalOp::Le => physical <= val,
            SignalOp::Eq => physical.to_bits() == val.to_bits(),
        };
        if !ok {
            return false;
        }
    }
    // Time window: if either bound is set, frames without a timestamp drop out.
    if f.time_from_ms.is_some() || f.time_to_ms.is_some() {
        let ts = match fj.timestamp_ms {
            Some(t) => t,
            None => return false,
        };
        if let Some(from) = f.time_from_ms {
            if ts < from {
                return false;
            }
        }
        if let Some(to) = f.time_to_ms {
            if ts > to {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::SignalValueJson;

    fn sig(name: &str, physical: f64) -> SignalValueJson {
        SignalValueJson {
            name: name.to_string(),
            raw: physical as i64,
            physical,
            enum_text: None,
            unit: String::new(),
        }
    }
    fn frame(can_id: u32, msg_name: Option<&str>, ts: Option<f64>, signals: Vec<SignalValueJson>) -> FrameJson {
        FrameJson {
            line_number: 1,
            timestamp_ms: ts,
            channel: 0,
            can_id,
            can_id_hex: format!("0x{can_id:X}"),
            dlc: 8,
            data_hex: String::new(),
            is_remote: false,
            is_fd: false,
            error_frame: false,
            msg_name: msg_name.map(|s| s.to_string()),
            signals,
        }
    }
    fn f() -> CanFilterJson {
        CanFilterJson::default()
    }

    #[test]
    fn empty_filter_keeps_all() {
        assert!(frame_matches(&frame(0x100, None, None, vec![]), &f()));
    }

    #[test]
    fn id_closed_range() {
        let mut flt = f();
        flt.id_from = Some(0x100);
        flt.id_to = Some(0x1ff);
        assert!(frame_matches(&frame(0x100, None, None, vec![]), &flt));
        assert!(frame_matches(&frame(0x1ff, None, None, vec![]), &flt));
        assert!(!frame_matches(&frame(0x200, None, None, vec![]), &flt));
    }

    #[test]
    fn id_reversed_matches_none() {
        let mut flt = f();
        flt.id_from = Some(0x200);
        flt.id_to = Some(0x100);
        assert!(!frame_matches(&frame(0x180, None, None, vec![]), &flt));
    }

    #[test]
    fn msg_name_exact() {
        let mut flt = f();
        flt.msg_name = Some("A".into());
        assert!(frame_matches(&frame(1, Some("A"), None, vec![]), &flt));
        assert!(!frame_matches(&frame(1, Some("B"), None, vec![]), &flt));
        assert!(!frame_matches(&frame(1, None, None, vec![]), &flt), "no msg dropped when set");
    }

    #[test]
    fn signal_gt_and_missing() {
        let mut flt = f();
        flt.signal_name = Some("Speed".into());
        flt.signal_op = Some(SignalOp::Gt);
        flt.signal_value = Some(100.0);
        assert!(frame_matches(&frame(1, Some("M"), None, vec![sig("Speed", 500.0)]), &flt));
        assert!(!frame_matches(&frame(1, Some("M"), None, vec![sig("Speed", 50.0)]), &flt));
        assert!(!frame_matches(&frame(1, Some("M"), None, vec![sig("Other", 500.0)]), &flt), "missing signal excludes");
    }

    #[test]
    fn signal_eq_object_is_semantics() {
        let mut flt = f();
        flt.signal_name = Some("S".into());
        flt.signal_op = Some(SignalOp::Eq);
        // NaN == NaN under Object.is (bit-equal), unlike f64 ==.
        flt.signal_value = Some(f64::NAN);
        assert!(frame_matches(&frame(1, Some("M"), None, vec![sig("S", f64::NAN)]), &flt), "Object.is(NaN,NaN) true");
        flt.signal_value = Some(5.0);
        assert!(frame_matches(&frame(1, Some("M"), None, vec![sig("S", 5.0)]), &flt));
        assert!(!frame_matches(&frame(1, Some("M"), None, vec![sig("S", 5.0001)]), &flt));
    }

    #[test]
    fn time_window_no_ts_drops() {
        let mut flt = f();
        flt.time_from_ms = Some(0.0);
        flt.time_to_ms = Some(1000.0);
        assert!(frame_matches(&frame(1, None, Some(500.0), vec![]), &flt));
        assert!(!frame_matches(&frame(1, None, Some(2000.0), vec![]), &flt));
        assert!(!frame_matches(&frame(1, None, None, vec![]), &flt), "ts-less dropped when window active");
    }

    #[test]
    fn axes_combine_with_and() {
        let mut flt = f();
        flt.id_from = Some(0x100);
        flt.id_to = Some(0x1ff);
        flt.msg_name = Some("EngineData".into());
        flt.signal_name = Some("Speed".into());
        flt.signal_op = Some(SignalOp::Ge);
        flt.signal_value = Some(60.0);
        assert!(frame_matches(&frame(0x123, Some("EngineData"), Some(10.0), vec![sig("Speed", 60.0)]), &flt));
        assert!(!frame_matches(&frame(0x999, Some("EngineData"), Some(10.0), vec![sig("Speed", 99.0)]), &flt), "id out");
    }
}
