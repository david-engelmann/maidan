//! How long a claim taken with `claim_next_thread` is leased for.
//!
//! A claim with no lease never comes back: if its holder dies, the thread
//! stays assigned to an agent that is not running and nobody else can take
//! it. So every `claim_next_thread` claim is leased. A caller that leaves out
//! `lease_secs` gets the server's default (`MAIDAN_CLAIM_DEFAULT_LEASE_SECS`,
//! 600 s), and the holder keeps the thread by renewing before the lease runs
//! out. A requested lease must be between 1 second and 7 days; renewals are
//! held to the same bounds.
//!
//! REST and MCP both resolve the lease here, so the two surfaces cannot
//! disagree.

use thiserror::Error;

/// The lease a `claim_next_thread` claim gets when the caller names none.
pub const DEFAULT_CLAIM_LEASE_SECS: i64 = 600;

/// The longest lease a claim or a renewal may ask for (7 days). Longer work
/// renews; an unbounded lease is a claim that never comes back.
pub const MAX_CLAIM_LEASE_SECS: i64 = 7 * 24 * 60 * 60;

/// A requested lease outside `1..=MAX_CLAIM_LEASE_SECS`.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("lease_secs must be between 1 and {MAX_CLAIM_LEASE_SECS} seconds, got {0}")]
pub struct InvalidLease(pub i64);

/// Check a requested lease (a claim's or a renewal's) against the bounds.
pub fn check_lease_secs(secs: i64) -> Result<i64, InvalidLease> {
    if (1..=MAX_CLAIM_LEASE_SECS).contains(&secs) {
        Ok(secs)
    } else {
        Err(InvalidLease(secs))
    }
}

/// The server's claim-lease policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClaimLeasePolicy {
    default_secs: i64,
}

impl ClaimLeasePolicy {
    /// A policy whose default lease is `default_secs`; it must itself be a
    /// valid lease.
    pub fn new(default_secs: i64) -> Result<Self, InvalidLease> {
        Ok(Self {
            default_secs: check_lease_secs(default_secs)?,
        })
    }

    /// Read `MAIDAN_CLAIM_DEFAULT_LEASE_SECS`. An unset or invalid value keeps
    /// [`DEFAULT_CLAIM_LEASE_SECS`], and an invalid one is logged.
    pub fn from_env() -> Self {
        Self::from_raw(std::env::var("MAIDAN_CLAIM_DEFAULT_LEASE_SECS").ok())
    }

    fn from_raw(raw: Option<String>) -> Self {
        let Some(raw) = raw else {
            return Self::default();
        };
        match raw.trim().parse::<i64>().map(Self::new) {
            Ok(Ok(policy)) => policy,
            _ => {
                tracing::warn!(
                    value = %raw,
                    default = DEFAULT_CLAIM_LEASE_SECS,
                    "MAIDAN_CLAIM_DEFAULT_LEASE_SECS is not a lease between 1 s and 7 days; using the default"
                );
                Self::default()
            }
        }
    }

    /// The default lease, in seconds.
    pub fn default_secs(&self) -> i64 {
        self.default_secs
    }

    /// The lease a claim gets: the one asked for, checked, or the default.
    pub fn lease_for(&self, requested: Option<i64>) -> Result<i64, InvalidLease> {
        requested.map_or(Ok(self.default_secs), check_lease_secs)
    }
}

impl Default for ClaimLeasePolicy {
    fn default() -> Self {
        Self {
            default_secs: DEFAULT_CLAIM_LEASE_SECS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_without_a_lease_gets_the_default() {
        let policy = ClaimLeasePolicy::new(90).expect("valid");
        assert_eq!(policy.lease_for(None), Ok(90));
        assert_eq!(policy.lease_for(Some(30)), Ok(30));
    }

    #[test]
    fn a_lease_outside_one_second_to_seven_days_is_refused() {
        let policy = ClaimLeasePolicy::default();
        assert_eq!(policy.lease_for(Some(0)), Err(InvalidLease(0)));
        assert_eq!(policy.lease_for(Some(-1)), Err(InvalidLease(-1)));
        assert_eq!(
            policy.lease_for(Some(MAX_CLAIM_LEASE_SECS + 1)),
            Err(InvalidLease(MAX_CLAIM_LEASE_SECS + 1))
        );
        assert_eq!(
            policy.lease_for(Some(i64::MAX)),
            Err(InvalidLease(i64::MAX))
        );
        assert_eq!(policy.lease_for(Some(1)), Ok(1));
        assert_eq!(
            policy.lease_for(Some(MAX_CLAIM_LEASE_SECS)),
            Ok(MAX_CLAIM_LEASE_SECS)
        );
    }

    #[test]
    fn the_default_comes_from_the_environment_when_it_is_a_valid_lease() {
        assert_eq!(ClaimLeasePolicy::from_raw(None).default_secs(), 600);
        assert_eq!(
            ClaimLeasePolicy::from_raw(Some(" 120 ".into())).default_secs(),
            120
        );
        for bad in ["0", "-5", "soon", "604801"] {
            assert_eq!(
                ClaimLeasePolicy::from_raw(Some(bad.into())).default_secs(),
                DEFAULT_CLAIM_LEASE_SECS,
                "{bad}"
            );
        }
    }

    #[test]
    fn a_policy_cannot_default_to_an_invalid_lease() {
        assert_eq!(ClaimLeasePolicy::new(0), Err(InvalidLease(0)));
    }
}
