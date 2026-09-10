//! Background wait-timer sweeper (Cluster 364, G2/G4).
//!
//! Opt-in via `MAIDAN_WAIT_SWEEP_TICK_SECS` (>0). Each tick drains every thread
//! wait past its deadline: it **atomically** claims and fires the wait in the store
//! (`claim_next_due_wait` — `FOR UPDATE SKIP LOCKED` on Postgres, so concurrent
//! replicas never double-fire one wait) and then applies its escalation.
//!
//! The escalation is **never a decision** (no auto close/approve/decline — the
//! "TimedOut ≠ Decline" rule): it emits a `WaitTimedOut` event (the notification
//! router then reaches the thread's owner) and, for the `Park` policy, additionally
//! marks the thread unclaimable (Cluster 363) so `claim_next` won't dispatch a
//! stuck thread until a human intervenes.

use std::time::Duration;

use maidan_types::{EscalationPolicy, Event, ThreadWait};

use crate::state::AppState;

/// Belt-and-suspenders bound on firings per tick, so a large due backlog can't
/// do unbounded work in one pass; the remainder fires on later ticks.
const MAX_FIRINGS_PER_TICK: u32 = 1000;

#[derive(Debug, Clone)]
pub struct WaitSweeperConfig {
    pub tick: Duration,
}

/// Build the config from the environment, or `None` when `MAIDAN_WAIT_SWEEP_TICK_SECS`
/// is unset / non-positive (the sweeper is not started — waits simply never fire
/// until an operator enables it).
pub fn config_from_env() -> Option<WaitSweeperConfig> {
    let secs = std::env::var("MAIDAN_WAIT_SWEEP_TICK_SECS")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|&s| s > 0)?;
    Some(WaitSweeperConfig {
        tick: Duration::from_secs(secs),
    })
}

/// Fire every wait due at "now": claim + fire atomically, then escalate. Returns
/// the number fired (for tests / logging).
pub async fn sweep_once(state: &AppState) -> u32 {
    let now = chrono::Utc::now();
    let mut fired = 0u32;
    while fired < MAX_FIRINGS_PER_TICK {
        let wait = match state.store.claim_next_due_wait(now).await {
            Ok(Some(w)) => w,
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(error = %err, "wait sweeper: claim failed");
                break;
            }
        };
        fired += 1;
        escalate(state, &wait).await;
    }
    fired
}

/// Apply a fired wait's escalation policy. Best-effort: a store hiccup on one wait
/// is logged, not fatal to the sweep.
async fn escalate(state: &AppState, wait: &ThreadWait) {
    // Resolve the thread's channel + workspace for the event (the wait row carries
    // only the thread id).
    let thread = match state.store.get_thread(wait.thread_id).await {
        Ok(t) => t,
        Err(err) => {
            tracing::warn!(error = %err, "wait sweeper: thread lookup failed");
            return;
        }
    };
    let channel = match state.store.get_channel(thread.channel_id).await {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(error = %err, "wait sweeper: channel lookup failed");
            return;
        }
    };

    // Park before announcing, so a subscriber that reacts to `WaitTimedOut` sees
    // the thread already parked.
    if wait.on_timeout == EscalationPolicy::Park {
        let reason = match &wait.reason {
            Some(r) => format!("wait timed out: {r}"),
            None => "wait timed out".to_string(),
        };
        if let Err(err) = state
            .store
            .mark_thread_unclaimable(wait.thread_id, &reason, wait.created_by)
            .await
        {
            tracing::warn!(error = %err, "wait sweeper: park failed");
        }
    }

    // Derived, standalone signal — no domain row to be atomic with, so `publish`
    // (the notification router notifies the owner off this).
    crate::routes::publish(
        state,
        Event::WaitTimedOut {
            occurred_at: chrono::Utc::now(),
            workspace_id: channel.workspace_id,
            channel_id: thread.channel_id,
            thread_id: wait.thread_id,
            policy: wait.on_timeout.as_str().to_string(),
            reason: wait.reason.clone(),
        },
    )
    .await;
    crate::metrics::record_wait_timed_out(wait.on_timeout.as_str());
}

/// The sweeper loop (spawned in `main.rs` when configured).
pub async fn run(state: AppState, cfg: WaitSweeperConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "wait sweeper started");
    loop {
        sweep_once(&state).await;
        tokio::time::sleep(cfg.tick).await;
    }
}
