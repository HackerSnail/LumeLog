//! DBC data model (spec §7.1). The subset parser lives in `dbc::parse` (Task 2);
//! this module holds only the shared types for the rest of the crate and, later,
//! the IPC layer.

/// Signal bit order, from the DBC `@` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// Intel / little-endian (`@1`): `start_bit` is the LSB.
    LittleEndian,
    /// Motorola / big-endian (`@0`): `start_bit` is the MSB in DBC sawtooth
    /// numbering. Algorithm pinned against cantools in Task 3.
    BigEndian,
}

/// Multiplexing role of a signal within its message (spec §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mux {
    /// Plain signal, always present.
    NonMuxed,
    /// `M` — carries the multiplexor selector value.
    Selector,
    /// `m<n>` — present only when the selector equals `n`.
    Indexed(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub name: String,
    pub start_bit: u32,
    pub length: u32,
    pub byte_order: ByteOrder,
    pub is_signed: bool,
    pub factor: f64,
    pub offset: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub unit: String,
    pub receivers: Vec<String>,
    /// `VAL_` enum entries `(raw, text)`; empty when the signal has none.
    pub value_table: Vec<(i64, String)>,
    pub mux: Mux,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: u32,
    pub name: String,
    pub dlc: u8,
    pub transmitter: String,
    /// Set when the raw DBC id had bit31 raised; `id` is the masked 29-bit value.
    pub is_extended: bool,
    pub signals: Vec<Signal>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Database {
    pub version: String,
    pub nodes: Vec<String>,
    pub messages: Vec<Message>,
}
