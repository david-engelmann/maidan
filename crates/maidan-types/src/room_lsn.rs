//! Event-log high-water (`maidan_events.id`) — projector / broadcast lag.
//!
//! This is **not** the Postgres WAL [`crate::Lsn`] used by
//! `Maidan-Consistency-Token`. Different name, value space (decimal i64 vs
//! `high/low` hex WAL), gating (always on, including SQLite), and purpose
//! (clients compare last-seen `log_id` to the room head). Do not parse one as
//! the other.

use serde::{Deserialize, Serialize};

/// HTTP / WS / MCP / A2A / projector header carrying the room high-water.
/// Wire form is `Maidan-Room-LSN` (HTTP header names are case-insensitive).
pub const ROOM_LSN_HEADER: &str = "maidan-room-lsn";

/// Highest `maidan_events.id` the server has committed (`0` when the log is
/// empty). Distinct from [`crate::Lsn`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RoomLsn(pub i64);

impl RoomLsn {
    /// Empty log (or unknown). Clients treat this as "no events yet".
    pub const EMPTY: Self = Self(0);

    /// Wrap a `MAX(id)` result. Negative ids do not occur; clamp anyway so a
    /// header is always a non-negative decimal.
    pub fn from_max_id(id: i64) -> Self {
        Self(id.max(0))
    }

    pub fn as_i64(self) -> i64 {
        self.0
    }

    /// Decimal form for the header (never WAL `0/hex`).
    pub fn to_header_str(self) -> String {
        self.0.to_string()
    }

    /// Parse a decimal header. Rejects WAL text (`0/3000128`) so the two
    /// tokens cannot be conflated.
    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.contains('/') {
            return None;
        }
        trimmed.parse::<i64>().ok().filter(|&n| n >= 0).map(Self)
    }
}

impl std::fmt::Display for RoomLsn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Lsn;

    #[test]
    fn decimal_round_trips_and_rejects_wal_text() {
        assert_eq!(RoomLsn::parse("42"), Some(RoomLsn(42)));
        assert_eq!(RoomLsn::parse(" 0 "), Some(RoomLsn::EMPTY));
        assert_eq!(RoomLsn::parse("-1"), None);
        assert_eq!(RoomLsn::parse("0/3000128"), None);
        assert_eq!(RoomLsn::parse("not-a-number"), None);
        let lsn = Lsn::from_pg_str("0/3000128").expect("wal");
        assert_eq!(RoomLsn::parse(&lsn.to_pg_str()), None);
        assert_ne!(RoomLsn(42).to_header_str(), lsn.to_pg_str());
    }

    #[test]
    fn from_max_id_clamps_negative() {
        assert_eq!(RoomLsn::from_max_id(7).as_i64(), 7);
        assert_eq!(RoomLsn::from_max_id(-3), RoomLsn::EMPTY);
    }
}
