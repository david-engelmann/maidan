//! Background scheduled/recurring-task sweeper.
//!
//! Opt-in via `MAIDAN_SCHEDULER_TICK_SECS` (>0). Each tick drains every
//! schedule that is due (`active AND next_run_at <= now`): it **atomically**
//! claims and advances the schedule in the store (`claim_next_due_schedule` —
//! `FOR UPDATE SKIP LOCKED` on Postgres, so concurrent replicas never
//! double-fire one schedule) and then creates the task thread. The claim
//! commits before the thread is created, so a crash in between drops that one
//! firing (at-most-once) rather than duplicating it.
//!
//! A recurring schedule re-arms to `now + interval` (fire-once-per-tick — no
//! catch-up storm when a schedule is far overdue); a one-shot deactivates.

use std::time::Duration;

use maidan_types::{Event, NewThread, TaskSchedule};

use crate::registration_owner::NotActing;
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
        // Claimed and advanced, so a suspended workspace's run is skipped,
        // not owed: it does not fire a backlog when the suspension lifts.
        if crate::suspension::held_back(state, sched.workspace_id).await {
            continue;
        }
        // A schedule opens threads as its creator, so it fires only while
        // the creator could open them itself (SEC-018).
        if let Err(not_acting) = creator_may_fire(state, &sched).await {
            skip(state, &sched, not_acting).await;
            continue;
        }
        match sched.recipe_id {
            Some(_) => fire_recipe(state, &sched).await,
            None => fire_bare_thread(state, &sched).await,
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

/// Whether `sched`'s creator may still open a thread in its channel: the
/// member is not removed or deactivated, and is still in the channel if it is
/// private. Ending a member's authority pauses their schedules; this also
/// covers a member who left a private channel since, and one ended another
/// way.
async fn creator_may_fire(state: &AppState, sched: &TaskSchedule) -> Result<(), NotActing> {
    let store = state.store.as_ref();
    match store.get_member(sched.created_by).await {
        Ok(member) if member.tombstoned_at.is_none() => {}
        Ok(_) | Err(maidan_store::StoreError::NotFound) => {
            return Err(NotActing::Ended("creator removed"));
        }
        Err(_) => return Err(NotActing::Held("creator check failed")),
    }
    match store.get_scim_user(sched.created_by).await {
        Ok(Some(user)) if !user.active => return Err(NotActing::Ended("creator deactivated")),
        Ok(_) => {}
        Err(_) => return Err(NotActing::Held("creator check failed")),
    }
    match maidan_auth::member_can_access_channel(
        store,
        sched.workspace_id,
        sched.created_by,
        sched.channel_id,
    )
    .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(NotActing::Ended("creator is not in the channel")),
        Err(_) => Err(NotActing::Held("creator check failed")),
    }
}

/// Record a skipped firing. A schedule whose creator's authority has ended is
/// paused, so it does not skip, and record, on every tick from now on; one
/// whose check could not finish only misses this run.
async fn skip(state: &AppState, sched: &TaskSchedule, not_acting: NotActing) {
    crate::metrics::record_task_schedule_fired("skipped");
    let paused = matches!(not_acting, NotActing::Ended(_))
        && state
            .store
            .set_task_schedule_active(sched.id, false)
            .await
            .is_ok();
    crate::registration_owner::record_skip(
        state,
        sched.workspace_id,
        "task_schedule",
        Some(sched.id.0),
        not_acting.reason(),
        serde_json::json!({ "created_by": sched.created_by.0, "paused": paused }),
    )
    .await;
}

/// Fire a plain schedule: create one titled thread in its channel.
async fn fire_bare_thread(state: &AppState, sched: &TaskSchedule) {
    match state
        .store
        .create_thread_with_event(NewThread {
            channel_id: sched.channel_id,
            parent_thread_id: None,
            title: Some(sched.title.clone()),
            description: None,
            created_by: Some(sched.created_by),
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

/// Fire a recipe-backed schedule: instantiate the recipe (a parent thread with
/// DAG children, copy-on-fire), unless its previous run is still in flight — in
/// which case skip and emit `ScheduleSkipped` so the run doesn't pile up. A
/// dangling `recipe_id` (a recipe deleted under a SQLite schedule) falls back
/// to a bare thread. Fires param-less; a recipe with required params can't be
/// scheduled (schedule-level params are a follow-up).
async fn fire_recipe(state: &AppState, sched: &TaskSchedule) {
    let Some(recipe_id) = sched.recipe_id else {
        return;
    };
    let recipe = match state.store.get_recipe(recipe_id).await {
        Ok(r) => r,
        Err(_) => {
            // The recipe is gone (SQLite has no FK); fall back to a bare thread.
            fire_bare_thread(state, sched).await;
            return;
        }
    };

    // Skip when the previous run's root thread hasn't reached a terminal state.
    if let Ok(Some(prev)) = state.store.latest_recipe_run(recipe_id).await
        && let Ok(root) = state.store.get_thread(prev.root_thread_id).await
        && !root.state.is_terminal()
    {
        let event = Event::ScheduleSkipped {
            occurred_at: chrono::Utc::now(),
            workspace_id: recipe.workspace_id,
            channel_id: recipe.channel_id,
            schedule_id: sched.id,
            recipe_id,
            reason: format!("previous run still in flight ({})", root.state.as_str()),
        };
        crate::routes::publish(state, event).await;
        crate::metrics::record_task_schedule_fired("skipped");
        tracing::info!(schedule = %sched.id, %recipe_id, "scheduler skipped: prior run in flight");
        return;
    }

    match state
        .store
        .instantiate_recipe(recipe_id, serde_json::Value::Null, sched.created_by)
        .await
    {
        Ok((run, events)) => {
            for stored in events {
                crate::routes::publish_stored(state, stored).await;
            }
            crate::metrics::record_task_schedule_fired("created");
            tracing::info!(schedule = %sched.id, %recipe_id, root = %run.root_thread_id, "scheduler fired recipe");
        }
        Err(err) => {
            crate::metrics::record_task_schedule_fired("failed");
            tracing::warn!(error = %err, schedule = %sched.id, %recipe_id, "scheduler: recipe instantiate failed");
        }
    }
}

/// Loop: sweep, then sleep `cfg.tick`. Spawned once at startup when configured.
pub async fn run(state: AppState, cfg: SchedulerConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "scheduler sweeper started");
    let _watched = crate::ops_alert::start_loop("scheduler", cfg.tick);
    loop {
        crate::ops_alert::guarded_tick("scheduler", sweep_once(&state)).await;
        tokio::time::sleep(cfg.tick).await;
    }
}
