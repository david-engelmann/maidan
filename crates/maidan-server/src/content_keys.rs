//! Server wiring for crypto-shredding: the startup rewrap that finishes a KEK
//! rotation. The keyring itself comes from
//! [`maidan_store::content_keyring::from_env`].

use std::sync::Arc;
use std::time::Duration;

use std::future::Future;

use maidan_store::{RewrapPass, Store, StoreError};

/// Rows per transaction in each [`spawn_rewrap`] pass.
const REWRAP_BATCH: i64 = 500;

/// The longest [`spawn_rewrap`] waits between stalled or failed passes, and
/// how often it looks again once nothing is left.
const MAX_RETRY: Duration = Duration::from_secs(600);

/// The next wait after `delay`: doubled, at most [`MAX_RETRY`].
fn back_off(delay: Duration) -> Duration {
    delay.saturating_mul(2).min(MAX_RETRY)
}

/// Rewrap every live content key still wrapped by a previous KEK or a retired
/// workspace key, for as long as the server runs. A failed pass, or one left
/// only with rows the custody refuses (counted in
/// `maidan_content_keys_refused`), is retried after `retry`, doubling up to
/// ten minutes until a pass makes progress. Once nothing is left it looks
/// again every ten minutes, so a workspace-key rotation made after boot (by
/// the CLI, whose own rewrap can stop on refused rows) is finished without a
/// restart. Replicas running this concurrently are safe: each page re-reads
/// under lock.
pub fn spawn_rewrap(store: Arc<dyn Store>, retry: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        run_rewrap(
            || {
                let store = store.clone();
                async move { store.content_keys_needing_rewrap().await }
            },
            || {
                let store = store.clone();
                async move { store.rewrap_content_keys(REWRAP_BATCH).await }
            },
            tokio::time::sleep,
            retry,
        )
        .await
    })
}

/// The [`spawn_rewrap`] loop, with the store calls and the clock passed in so
/// the delay policy can be tested without either.
async fn run_rewrap<C, CF, P, PF, S, SF>(mut count: C, mut pass: P, mut sleep: S, retry: Duration)
where
    C: FnMut() -> CF,
    CF: Future<Output = Result<u64, StoreError>>,
    P: FnMut() -> PF,
    PF: Future<Output = Result<RewrapPass, StoreError>>,
    S: FnMut(Duration) -> SF,
    SF: Future<Output = ()>,
{
    loop {
        // Idle: wait for work before running passes.
        match count().await {
            Ok(0) => {
                sleep(MAX_RETRY).await;
                continue;
            }
            Ok(pending) => {
                tracing::info!(pending, "content keys: rewrapping under the active keys")
            }
            Err(err) => tracing::warn!(error = %err, "content keys: rewrap count failed"),
        }
        let mut total = 0u64;
        let mut delay = retry;
        loop {
            match pass().await {
                Ok(pass) => {
                    total += pass.changed;
                    metrics::gauge!("maidan_content_keys_refused").set(pass.skipped as f64);
                    if pass.changed > 0 {
                        delay = retry;
                        continue;
                    }
                    if pass.skipped == 0 {
                        break;
                    }
                    // Only rows the custody refuses are left; they stay
                    // on their old keys until it allows them.
                    tracing::warn!(
                        refused = pass.skipped,
                        retry_in_secs = delay.as_secs(),
                        "content keys: rewrap stalled on rows the custody refuses; retrying"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        retry_in_secs = delay.as_secs(),
                        "content keys: rewrap pass failed; retrying"
                    );
                }
            }
            sleep(delay).await;
            delay = back_off(delay);
        }
        tracing::info!(total, "content keys: rewrap complete");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// Run the loop over scripted pass results; return every wait it asked
    /// for, stopping once the script (and then one idle wait) runs out.
    async fn waits(counts: Vec<u64>, passes: Vec<Result<RewrapPass, ()>>) -> Vec<u64> {
        let counts = RefCell::new(VecDeque::from(counts));
        let passes = RefCell::new(VecDeque::from(passes));
        let slept = RefCell::new(Vec::new());
        let done = std::sync::atomic::AtomicBool::new(false);
        let run = run_rewrap(
            || {
                let next = counts.borrow_mut().pop_front();
                async move { Ok(next.unwrap_or(0)) }
            },
            || {
                let next = passes.borrow_mut().pop_front();
                async move {
                    match next {
                        Some(Ok(p)) => Ok(p),
                        Some(Err(())) => {
                            Err(StoreError::ContentKey(maidan_types::SealError::Unavailable))
                        }
                        None => Ok(RewrapPass::default()),
                    }
                }
            },
            |d: Duration| {
                slept.borrow_mut().push(d.as_secs());
                // A loop that never parks is a test failure, not an out-of-memory.
                assert!(slept.borrow().len() < 1_000, "run_rewrap never went idle");
                let idle =
                    d == MAX_RETRY && passes.borrow().is_empty() && counts.borrow().is_empty();
                if idle {
                    done.store(true, std::sync::atomic::Ordering::SeqCst);
                    // Park here, so the never-ending loop yields to the driver.
                    futures::future::Either::Left(std::future::pending())
                } else {
                    futures::future::Either::Right(std::future::ready(()))
                }
            },
            Duration::from_secs(30),
        );
        tokio::pin!(run);
        // The loop never ends; drive it until it goes idle with nothing left.
        std::future::poll_fn(|cx| {
            let _ = run.as_mut().poll(cx);
            if done.load(std::sync::atomic::Ordering::SeqCst) {
                std::task::Poll::Ready(())
            } else {
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        })
        .await;
        slept.borrow().clone()
    }

    fn pass(changed: u64, skipped: u64) -> Result<RewrapPass, ()> {
        Ok(RewrapPass { changed, skipped })
    }

    #[test]
    fn back_off_doubles_to_a_ten_minute_cap() {
        let mut d = Duration::from_secs(30);
        let mut seen = Vec::new();
        for _ in 0..7 {
            seen.push(d.as_secs());
            d = back_off(d);
        }
        assert_eq!(seen, [30, 60, 120, 240, 480, 600, 600]);
    }

    #[tokio::test]
    async fn stalls_double_to_the_cap_and_progress_resets_the_wait() {
        let script = vec![
            Err(()),
            pass(0, 3),
            Err(()),
            pass(0, 3),
            pass(0, 3),
            pass(0, 3),
            pass(0, 3),
            pass(2, 1), // progress: the next stall waits 30 s again
            pass(0, 1),
            pass(1, 0),
            pass(0, 0), // done: back to idle
        ];
        let slept = waits(vec![5], script).await;
        assert_eq!(slept, [30, 60, 120, 240, 480, 600, 600, 30, 600]);
    }

    #[tokio::test]
    async fn a_rotation_after_an_idle_spell_is_picked_up_without_a_restart() {
        // Nothing at boot, then a rotation leaves 4 rows to move.
        let slept = waits(vec![0, 0, 4], vec![pass(4, 0), pass(0, 0)]).await;
        assert_eq!(
            slept,
            [600, 600, 600],
            "idle checks, then idle again once moved"
        );
    }
}
