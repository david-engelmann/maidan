//! A retry budget per destination host, shared by the outbound delivery
//! workers.
//!
//! Each worker (webhooks, automation HTTP, projector and result egress, mail)
//! retries a failed delivery on its own exponential backoff. Backoff spreads one
//! row's attempts; it does nothing about many rows to the same host. When a
//! host that was down for a while comes back, every delivery queued for it is
//! due or overdue, and the workers send them all at once: the recovering host
//! takes its whole backlog in one burst and falls over again. That thundering
//! herd is what slowed the recovery in Cloudflare's November 2023 control-plane
//! outage, and it is why gRPC (retry throttling) and Finagle (`RetryBudget`)
//! bound retries separately from backoff.
//!
//! **Shape: a token bucket per host, spent only by retries.** Each host starts
//! with [`RETRY_BURST`] tokens and gains [`RETRY_REFILL_PER_SEC`] a second, up
//! to the burst. A retry (any attempt after a delivery's first) spends one; with
//! none left it is refused. A first attempt is never refused and spends
//! nothing: new traffic is not what storms, and holding it back would delay
//! deliveries that have not failed once. So in any window of `T` seconds a
//! host receives at most `RETRY_BURST + RETRY_REFILL_PER_SEC × T` retries from
//! this process, whatever the size of the backlog.
//!
//! A retry-ratio budget (retries at most N% of recent attempts, plus a floor)
//! was the alternative. It suits live request traffic, where retries amplify
//! the requests being made now. Here the retries come from a durable queue
//! whose size has nothing to do with current traffic, and in a storm there is
//! little first-attempt traffic to take a ratio of, so the budget would sit at
//! its floor anyway: a token bucket states that floor directly and gives an
//! operator one number to read.
//!
//! **Per host, across workers.** The key is the destination's host: the host of
//! a webhook or automation URL, the Slack or GitHub API host, the SMTP relay. A
//! webhook and an automation hook pointed at the same receiver share its
//! budget, because it is the receiver that recovers, not the queue. A host that
//! is down does not spend another host's budget.
//!
//! **Refused means deferred.** A refused retry is not sent, not failed and not
//! counted as an attempt: the worker moves its next attempt forward by
//! [`Admission::Defer`]'s delay and keeps its attempt count, so the budget can
//! never walk a row into the dead-letter queue. Delays are handed out one
//! refill interval apart, so a backlog comes back spread at about the rate the
//! bucket refills, clamped to [`MIN_DEFER`]..=[`MAX_DEFER`]. Every deferral is
//! one row update, so a backlog of `B` retries to one host costs at most about
//! `B / 60` updates a second while it waits.
//!
//! **In-process, per replica.** The buckets live in memory. N replicas give a
//! host N budgets, so the bound above is per replica, and a restart starts
//! every bucket full. Holding one budget across replicas would need shared
//! state on the send path of every delivery, which this does not add.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Retries a host may take at once, from a full bucket.
pub const RETRY_BURST: u32 = 10;

/// Retries a host regains per second, up to [`RETRY_BURST`].
pub const RETRY_REFILL_PER_SEC: u32 = 2;

/// The shortest deferral, so a refused row is not claimed again within the
/// same sweep.
pub const MIN_DEFER: Duration = Duration::from_secs(1);

/// The longest deferral: short next to the backoff a failure earns late in a
/// row's life (up to 256 s for webhooks and automation, an hour for mail and
/// egress), and long enough that a large backlog is not rewritten every
/// second while it waits.
pub const MAX_DEFER: Duration = Duration::from_secs(60);

/// Buckets held before idle ones are dropped. An idle bucket (full, nothing
/// deferred) behaves exactly like a missing one, so dropping it loses nothing.
const MAX_TRACKED_HOSTS: usize = 10_000;

/// Where a budget reads the time. Tests drive a [`ManualClock`].
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A clock that moves only when told to.
pub struct ManualClock {
    start: Instant,
    elapsed: Mutex<Duration>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
        }
    }

    pub fn advance(&self, by: Duration) {
        *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner) += by;
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.start + *self.elapsed.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Whether a send is a delivery's first attempt or a retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attempt {
    First,
    Retry,
}

impl Attempt {
    /// From the attempts a delivery has already made before this one.
    pub fn after(prior_attempts: i64) -> Self {
        if prior_attempts > 0 {
            Self::Retry
        } else {
            Self::First
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Send,
    /// Not now: try again after this long, without counting an attempt.
    Defer(Duration),
}

struct Bucket {
    tokens: f64,
    refilled_at: Instant,
    /// The latest time handed out to a deferred retry, so the next one is
    /// spaced after it instead of landing on the same instant.
    deferred_until: Instant,
}

pub struct RetryBudget {
    clock: Arc<dyn Clock>,
    burst: f64,
    refill_per_sec: f64,
    hosts: Mutex<HashMap<String, Bucket>>,
}

impl RetryBudget {
    /// The budget the server runs with: [`RETRY_BURST`] and
    /// [`RETRY_REFILL_PER_SEC`] on the system clock.
    pub fn new() -> Self {
        Self::with_clock(RETRY_BURST, RETRY_REFILL_PER_SEC, Arc::new(SystemClock))
    }

    /// A budget with its own limits and clock. A `refill_per_sec` of zero is
    /// treated as one, so a deferral always has a finite horizon.
    pub fn with_clock(burst: u32, refill_per_sec: u32, clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            burst: f64::from(burst),
            refill_per_sec: f64::from(refill_per_sec.max(1)),
            hosts: Mutex::new(HashMap::new()),
        }
    }

    /// Whether `attempt` to `host` may be sent now. A retry that is admitted
    /// spends a token; a first attempt is always admitted and spends nothing.
    pub fn admit(&self, host: &str, attempt: Attempt) -> Admission {
        if attempt == Attempt::First {
            return Admission::Send;
        }
        let now = self.clock.now();
        let key = host.to_ascii_lowercase();
        let mut hosts = self.hosts.lock().unwrap_or_else(PoisonError::into_inner);
        if !hosts.contains_key(&key) && hosts.len() >= MAX_TRACKED_HOSTS {
            hosts.retain(|_, bucket| !self.is_idle(bucket, now));
            if hosts.len() >= MAX_TRACKED_HOSTS {
                // Bounded memory over a bound on a host never seen before:
                // this takes more than ten thousand hosts retrying at once.
                tracing::warn!(host = %key, "retry budget: host table full; not tracking");
                return Admission::Send;
            }
        }
        let bucket = hosts.entry(key).or_insert_with(|| Bucket {
            tokens: self.burst,
            refilled_at: now,
            deferred_until: now,
        });
        self.refill(bucket, now);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            return Admission::Send;
        }
        let interval = Duration::from_secs_f64(1.0 / self.refill_per_sec);
        let next_token = now + Duration::from_secs_f64((1.0 - bucket.tokens) / self.refill_per_sec);
        let slot = if bucket.deferred_until > now {
            (bucket.deferred_until + interval).max(next_token)
        } else {
            next_token
        };
        let slot = slot.min(now + MAX_DEFER);
        bucket.deferred_until = slot;
        Admission::Defer(slot.duration_since(now).clamp(MIN_DEFER, MAX_DEFER))
    }

    fn refill(&self, bucket: &mut Bucket, now: Instant) {
        let elapsed = now.saturating_duration_since(bucket.refilled_at);
        bucket.tokens =
            (bucket.tokens + elapsed.as_secs_f64() * self.refill_per_sec).min(self.burst);
        bucket.refilled_at = now;
    }

    fn is_idle(&self, bucket: &Bucket, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(bucket.refilled_at);
        bucket.deferred_until <= now
            && bucket.tokens + elapsed.as_secs_f64() * self.refill_per_sec >= self.burst
    }
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self::new()
    }
}

/// The host a delivery URL is sent to: the budget's key. `None` for a URL that
/// does not parse, which fails before it reaches any host.
pub fn host_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    parsed.host_str().map(str::to_ascii_lowercase)
}

/// Ask `budget` about one attempt by `worker` to `host`. `None` means send it
/// now; `Some(t)` means the retry is deferred and due again at `t`, and the
/// deferral has been counted in `maidan_egress_retry_deferred_total`.
pub fn deferred_until(
    budget: &RetryBudget,
    worker: &'static str,
    host: &str,
    attempt: Attempt,
) -> Option<chrono::DateTime<chrono::Utc>> {
    match budget.admit(host, attempt) {
        Admission::Send => None,
        Admission::Defer(delay) => {
            crate::metrics::record_retry_deferred(worker);
            let delay = chrono::Duration::from_std(delay)
                .unwrap_or_else(|_| chrono::Duration::seconds(MAX_DEFER.as_secs() as i64));
            Some(chrono::Utc::now() + delay)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(burst: u32, refill: u32) -> (RetryBudget, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new());
        (RetryBudget::with_clock(burst, refill, clock.clone()), clock)
    }

    fn sent(budget: &RetryBudget, host: &str, attempt: Attempt, n: usize) -> usize {
        (0..n)
            .filter(|_| budget.admit(host, attempt) == Admission::Send)
            .count()
    }

    #[test]
    fn retries_past_the_burst_are_refused() {
        let (budget, _clock) = budget(5, 1);
        assert_eq!(sent(&budget, "hooks.example.com", Attempt::Retry, 5), 5);
        assert!(matches!(
            budget.admit("hooks.example.com", Attempt::Retry),
            Admission::Defer(_)
        ));
    }

    #[test]
    fn first_attempts_are_never_refused_and_spend_nothing() {
        let (budget, _clock) = budget(2, 1);
        assert_eq!(
            sent(&budget, "hooks.example.com", Attempt::First, 1000),
            1000
        );
        assert_eq!(
            sent(&budget, "hooks.example.com", Attempt::Retry, 2),
            2,
            "a thousand first attempts left the retry budget untouched"
        );
        assert!(matches!(
            budget.admit("hooks.example.com", Attempt::Retry),
            Admission::Defer(_)
        ));
        assert_eq!(
            budget.admit("hooks.example.com", Attempt::First),
            Admission::Send,
            "an empty retry budget does not hold back a first attempt"
        );
    }

    #[test]
    fn the_budget_refills_with_time_up_to_the_burst() {
        let (budget, clock) = budget(4, 2);
        assert_eq!(sent(&budget, "h", Attempt::Retry, 10), 4);

        clock.advance(Duration::from_millis(500));
        assert_eq!(
            sent(&budget, "h", Attempt::Retry, 10),
            1,
            "half a second at 2/s"
        );

        clock.advance(Duration::from_secs(1));
        assert_eq!(sent(&budget, "h", Attempt::Retry, 10), 2);

        clock.advance(Duration::from_secs(3600));
        assert_eq!(
            sent(&budget, "h", Attempt::Retry, 10),
            4,
            "an idle hour refills to the burst, not beyond it"
        );
    }

    #[test]
    fn each_host_has_its_own_budget() {
        let (budget, _clock) = budget(3, 1);
        assert_eq!(sent(&budget, "down.example.com", Attempt::Retry, 10), 3);
        assert_eq!(
            sent(&budget, "up.example.com", Attempt::Retry, 3),
            3,
            "draining one host does not spend another's"
        );
        assert_eq!(
            sent(&budget, "DOWN.example.com", Attempt::Retry, 1),
            0,
            "host names compare without case"
        );
    }

    #[test]
    fn a_deferral_is_due_its_delay_after_the_moment_it_is_made() {
        let (budget, _clock) = budget(1, 2);
        assert_eq!(budget.admit("h", Attempt::Retry), Admission::Send);
        let before = chrono::Utc::now();
        let until =
            deferred_until(&budget, "test", "h", Attempt::Retry).expect("the bucket is empty");
        let after = chrono::Utc::now();
        // The first deferral from an empty bucket is exactly MIN_DEFER.
        let delay = chrono::Duration::from_std(MIN_DEFER).expect("a small duration");
        assert!(
            before + delay <= until && until <= after + delay,
            "{until} is not {delay} after a moment between {before} and {after}"
        );
    }

    #[test]
    fn deferrals_are_spread_one_refill_interval_apart_and_bounded() {
        let (budget, _clock) = budget(1, 2);
        assert_eq!(budget.admit("h", Attempt::Retry), Admission::Send);
        let delays: Vec<Duration> = (0..200)
            .map(|_| match budget.admit("h", Attempt::Retry) {
                Admission::Defer(d) => d,
                Admission::Send => panic!("the bucket is empty"),
            })
            .collect();
        assert!(delays.iter().all(|d| (MIN_DEFER..=MAX_DEFER).contains(d)));
        // 0.5 s, 1.0 s, 1.5 s, ... before clamping.
        assert_eq!(delays[0], MIN_DEFER);
        assert_eq!(delays[2], Duration::from_millis(1500));
        assert_eq!(delays[3], Duration::from_secs(2));
        assert!(delays.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(*delays.last().unwrap(), MAX_DEFER);
    }

    #[test]
    fn a_refused_retry_does_not_spend_a_token() {
        let (budget, clock) = budget(1, 1);
        assert_eq!(budget.admit("h", Attempt::Retry), Admission::Send);
        for _ in 0..50 {
            assert!(matches!(
                budget.admit("h", Attempt::Retry),
                Admission::Defer(_)
            ));
        }
        clock.advance(Duration::from_secs(1));
        assert_eq!(
            budget.admit("h", Attempt::Retry),
            Admission::Send,
            "fifty refusals did not borrow against the next token"
        );
    }

    #[test]
    fn attempt_after_counts_prior_attempts() {
        assert_eq!(Attempt::after(0), Attempt::First);
        assert_eq!(Attempt::after(1), Attempt::Retry);
    }

    #[test]
    fn host_of_reads_the_url_host() {
        assert_eq!(
            host_of("https://Hooks.Example.com:8443/in?x=1").as_deref(),
            Some("hooks.example.com")
        );
        assert_eq!(host_of("not a url"), None);
    }
}
