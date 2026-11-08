//! The boot grace on lapsed claims.
//!
//! A server restart (a deploy) leaves it unreachable for minutes, longer than
//! a short lease. Every holder misses its heartbeats through no fault of its
//! own, and the first reaper tick after boot used to free every one of those
//! leases: each restart lost every claim in flight, the holder's result was
//! refused with `ClaimLost`, and the work was done again.
//!
//! So for a while after the server starts no lease counts as lapsed: the
//! reaper frees none (`reap_expired_claims` returns nothing), `claim_next`
//! takes none over, and the queue counts agree with `claim_next`. Renewal
//! and the result fence read the same lease clock, so a holder that comes
//! back within the grace renews and finishes as if the outage had not
//! happened. A holder that never comes back loses its claim when the grace
//! ends: from then on its lapsed lease is refused (`ClaimLost`) whether or
//! not the reaper or another claimer has reached it yet.
//!
//! The grace is this process's: each replica holds from its own start, and
//! nothing is stored.

use std::sync::atomic::{AtomicI64, Ordering};

use chrono::{DateTime, Utc};

/// When this process stops holding lapsed leases. Shared by the store and
/// the server, which starts it at boot (`MAIDAN_CLAIM_REAPER_BOOT_GRACE_SECS`).
#[derive(Debug, Default)]
pub struct ClaimGrace {
    /// Microseconds since the epoch until which no lease counts as lapsed;
    /// 0 when there is no grace, as before any is set.
    until_micros: AtomicI64,
}

impl ClaimGrace {
    /// Hold every lapsed lease until `until`. A later call replaces the
    /// deadline; one in the past ends the grace.
    pub fn hold_until(&self, until: DateTime<Utc>) {
        self.until_micros
            .store(until.timestamp_micros(), Ordering::Relaxed);
    }

    /// When the grace ends, or `None` when none was set.
    pub fn held_until(&self) -> Option<DateTime<Utc>> {
        match self.until_micros.load(Ordering::Relaxed) {
            0 => None,
            micros => DateTime::from_timestamp_micros(micros),
        }
    }

    /// True while lapsed leases are held at `now`.
    pub fn holding(&self, now: DateTime<Utc>) -> bool {
        now.timestamp_micros() < self.until_micros.load(Ordering::Relaxed)
    }

    /// The time a lease deadline is compared with to decide whether it
    /// lapsed: `now`, or during the grace the epoch, before every deadline,
    /// so every held lease reads as live.
    pub(crate) fn lease_clock(&self, now: DateTime<Utc>) -> DateTime<Utc> {
        if self.holding(now) {
            DateTime::<Utc>::UNIX_EPOCH
        } else {
            now
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_grace_the_lease_clock_is_now() {
        let grace = ClaimGrace::default();
        let now = Utc::now();
        assert!(!grace.holding(now));
        assert_eq!(grace.held_until(), None);
        assert_eq!(grace.lease_clock(now), now);
    }

    #[test]
    fn during_the_grace_no_lease_reads_as_lapsed_and_after_it_the_clock_is_now() {
        let grace = ClaimGrace::default();
        let now = Utc::now();
        let until = now + chrono::Duration::seconds(600);
        grace.hold_until(until);
        assert!(grace.holding(now));
        assert_eq!(grace.lease_clock(now), DateTime::<Utc>::UNIX_EPOCH);
        assert_eq!(
            grace.held_until().map(|t| t.timestamp_micros()),
            Some(until.timestamp_micros())
        );
        let after = until + chrono::Duration::seconds(1);
        assert!(!grace.holding(after));
        assert!(!grace.holding(until), "the grace ends at its deadline");
        assert_eq!(grace.lease_clock(after), after);
    }
}
