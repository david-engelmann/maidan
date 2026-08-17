//! Background scheduled/recurring-task sweeper (Cluster 227).
//!
//! Opt-in via `MAIDAN_SCHEDULER_TICK_SECS` (>0). Each tick drains every schedule
//! that is due (`active AND next_run_at <= now`): it **atomically** claims and
//! advances the schedule in the store (`claim_next_due_schedule` — `FOR UPDATE
//! SKIP LOCKED` on Postgres, so concurrent replicas never double-fire one
//! schedule) and then creates the task thread. The claim commits before the
//! thread is created, so a crash in between drops that one firing (at-most-once)
//! rather than duplicating it.
//!
//! A recurring schedule re-arms to `now + interval` (fire-once-per-tick — no
//! catch-up storm when a schedule is far overdue); a one-shot deactivates.

use std::time::Duration;

use maidan_types::NewThread;

use crate::state::AppState;

/// Belt-and-suspenders bound on firings per tick, so a large due backlog can't
/// create unbounded threads in one pass; the remainder fires on later ticks.
const MAX_FIRINGS_PER_TICK: u32 = 1000;

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub tick: Duration,
}

/// Build the config from the environment, or `None` when `MAIDAN_SCHEDULER_TICK_SECS`
/// is unset / non-positive (the sweeper is not started — schedules simply never
/// fire until an operator enables it).
pub fn config_from_env() -> Option<SchedulerConfig> {
    let secs = std::env::var("MAIDAN_SCHEDULER_TICK_SECS")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|&s| s > 0)?;
    Some(SchedulerConfig {
        tick: Duration::from_secs(secs),
    })
}

/// Fire every schedule due at "now": claim + advance atomically, create the task
/// thread, publish. Returns the number fired (for tests / logging).
pub async fn sweep_once(state: &AppState) -> u32 {
    let now = chrono::Utc::now();
    let mut fired = 0u32;
    while fired < MAX_FIRINGS_PER_TICK {
        let sched = match state.store.claim_next_due_schedule(now).await {
            Ok(Some(s)) => s,
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(error = %err, "scheduler: claim failed");
                break;
            }
        };
        fired += 1;
        match state
            .store
            .create_thread_with_event(NewThread {
                channel_id: sched.channel_id,
                parent_thread_id: None,
                title: Some(sched.title.clone()),
            })
            .await
        {
            Ok((thread, stored)) => {
                crate::routes::publish_stored(state, stored).await;
                crate::metrics::record_task_schedule_fired("created");
                tracing::info!(schedule = %sched.id, thread = %thread.id, "scheduler fired");
            }
            Err(err) => {
                crate::metrics::record_task_schedule_fired("failed");
                tracing::warn!(error = %err, schedule = %sched.id, "scheduler: thread create failed");
            }
        }
    }
    if fired == MAX_FIRINGS_PER_TICK {
        tracing::warn!(
            cap = MAX_FIRINGS_PER_TICK,
            "scheduler: hit per-tick firing cap; remainder fires next tick"
        );
    }
    fired
}

/// Loop: sweep, then sleep `cfg.tick`. Spawned once at startup when configured.
pub async fn run(state: AppState, cfg: SchedulerConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "scheduler sweeper started");
    loop {
        sweep_once(&state).await;
        tokio::time::sleep(cfg.tick).await;
    }
}
