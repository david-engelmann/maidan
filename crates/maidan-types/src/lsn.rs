//! Postgres WAL log-sequence number (`pg_lsn`) — the read-replica causality token
//! (Cluster 261, Program D).
//!
//! A write returns the primary's LSN at commit; a later read echoes it, and the
//! router serves the read from a replica only once the replica's replay position
//! has reached that LSN (else it falls back to the primary). Storing the LSN as a
//! `u64` (not its `X/Y` text) is load-bearing: the text form does **not** order
//! correctly as a string (`0/9` vs `0/10`), but the numeric form does.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A Postgres WAL LSN. Ordering is numeric, so `Lsn` comparisons answer "has the
/// replica caught up to this write?" directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Lsn(pub u64);

impl Lsn {
    /// Parse the Postgres `pg_lsn` text form `high/low` (two hex halves), e.g.
    /// `"0/3000128"`. Returns `None` for anything that isn't `hex/hex`.
    pub fn from_pg_str(s: &str) -> Option<Self> {
        let (hi, lo) = s.trim().split_once('/')?;
        let hi = u64::from_str_radix(hi.trim(), 16).ok()?;
        let lo = u64::from_str_radix(lo.trim(), 16).ok()?;
        Some(Lsn((hi << 32) | (lo & 0xFFFF_FFFF)))
    }

    /// Render back to the Postgres `pg_lsn` text form `high/low` (upper-case hex),
    /// suitable for binding as `::pg_lsn` or carrying in a header/token.
    pub fn to_pg_str(self) -> String {
        format!("{:X}/{:X}", self.0 >> 32, self.0 & 0xFFFF_FFFF)
    }
}

impl fmt::Display for Lsn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_pg_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_pg_text_form() {
        let l = Lsn::from_pg_str("0/3000128").unwrap();
        assert_eq!(l.to_pg_str(), "0/3000128");
        assert_eq!(
            Lsn::from_pg_str("16/B374D848").unwrap().to_pg_str(),
            "16/B374D848"
        );
    }

    #[test]
    fn orders_numerically_not_lexically() {
        // The text `0/9` sorts AFTER `0/10` as a string but must be LESS as an LSN.
        assert!(Lsn::from_pg_str("0/9").unwrap() < Lsn::from_pg_str("0/10").unwrap());
        // And across the high half.
        assert!(Lsn::from_pg_str("1/0").unwrap() > Lsn::from_pg_str("0/FFFFFFFF").unwrap());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Lsn::from_pg_str("nope").is_none());
        assert!(Lsn::from_pg_str("").is_none());
        assert!(Lsn::from_pg_str("0/").is_none());
        assert!(Lsn::from_pg_str("/0").is_none());
    }

    #[test]
    fn caught_up_is_a_gte_comparison() {
        let token = Lsn::from_pg_str("0/3000128").unwrap();
        let behind = Lsn::from_pg_str("0/3000000").unwrap();
        let ahead = Lsn::from_pg_str("0/3000200").unwrap();
        assert!(behind < token, "replay behind the token -> not caught up");
        assert!(ahead >= token, "replay past the token -> caught up");
        assert!(token >= token, "exactly at the token -> caught up");
    }
}
