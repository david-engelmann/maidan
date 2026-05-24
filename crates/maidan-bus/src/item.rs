//! Items delivered on a bus subscription stream.

use maidan_types::{BusEnvelope, Event};

/// A filtered bus subscription yields events and may surface lag.
#[derive(Debug, Clone)]
pub enum BusItem {
    Event(Box<BusEnvelope>),
    /// The subscriber fell behind the broadcast buffer; replay from HTTP.
    Lagged {
        skipped: u64,
    },
}

impl BusItem {
    pub fn event(log_id: i64, event: Event) -> Self {
        Self::Event(Box::new(BusEnvelope { log_id, event }))
    }
}
