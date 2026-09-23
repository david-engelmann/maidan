//! Items delivered on a bus subscription stream.

use maidan_types::BusEnvelope;

/// A filtered bus subscription yields events and may surface lag.
#[derive(Debug, Clone)]
pub enum BusItem {
    Event(Box<BusEnvelope>),
    /// The subscriber fell behind the broadcast buffer; replay from HTTP.
    Lagged {
        skipped: u64,
    },
}
