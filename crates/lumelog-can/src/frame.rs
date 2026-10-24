//! Frame and decode-result data types (spec §7.3 / §7.4). Pure data holders;
//! line parsing lives in `source`, bit extraction in `decode` (Plan 2 Tasks 3+).

/// Per-frame flag bits carried through decode and on to the CanPanel (Plan 3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameFlags {
    pub is_fd: bool,
    pub brs: bool,
    pub esi: bool,
    pub is_remote: bool,
    pub is_error_frame: bool,
}

/// One decoded CAN/CAN-FD frame, tagged with the source line it came from so the
/// CanPanel can jump back into the editor (spec §7.3 / §7.4). `data` is
/// zero-padded past `dlc`.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub line_number: u64,
    /// Seconds; `None` when the source line carries no timestamp.
    pub timestamp: Option<f64>,
    pub channel: u8,
    pub can_id: u32,
    pub dlc: u8,
    pub data: [u8; 64],
    pub flags: FrameFlags,
}

/// A single signal's decoded value within a frame.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalValue {
    pub name: String,
    /// Raw integer bits, sign-extended (two's complement when `is_signed`).
    pub raw: i64,
    /// `raw * factor + offset`.
    pub physical: f64,
    /// Enum text when the raw value hits a `VAL_` table entry.
    pub enum_text: Option<String>,
    pub unit: String,
}

/// A frame plus its (optional) DBC-driven signal decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedFrame {
    pub frame: Frame,
    pub message_name: Option<String>,
    pub signals: Vec<SignalValue>,
}

/// Running tallies for a `FrameSource`: frames emitted vs. lines skipped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceStats {
    pub decoded: u64,
    pub skipped: u64,
}
