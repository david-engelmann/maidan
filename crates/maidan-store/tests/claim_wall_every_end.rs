//! Every way a claim ends charges the time it worked against `max_wall_secs`,
//! on both backends. A lapsed lease was already charged (to its deadline). A
//! release, an unassign, a reassignment, a freeze, a SCIM deactivation, a
//! budget stop and a close charge from the acknowledgement to that ending.
//! An unacknowledged claim still charges nothing. `claim_next` does not hand
//! out a thread that is already over any budget.

use std::sync::Arc;

use maidan_fsm::ThreadAction;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, BudgetLimits, BudgetPatch, ChannelId, EventKind, MemberId, MemberKind,
    NewAuditEvent, NewMember, NewThread, NewWorkspace, ThreadId, UsageDelta, WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;

struct Room {
    channel: ChannelId,
    holder: MemberId,
    other: MemberId,
    workspace: WorkspaceId,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(maidan_types::NewChannel {
            workspace_id: ws.id,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let mut members = Vec::new();
    for handle in ["holder", "other"] {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    Room {
        channel: channel.id,
        holder: members[0],
        other: members[1],
        workspace: ws.id,
    }
}

async fn task(store: &dyn Store, channel: ChannelId, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
        })
        .await
        .unwrap()
        .id
}

/// Claim and acknowledge. The returned lease is the one a release must present.
async fn work(
    store: &dyn Store,
    channel: ChannelId,
    member: MemberId,
) -> (ThreadId, maidan_types::ClaimLeaseId) {
    let held = store
        .claim_next_thread(channel, member, Some(3600))
        .await
        .unwrap()
        .expect("claimable");
    let lease = held.claim_lease_id.expect("lease");
    let acked = store
        .acknowledge_claim(held.id, member, lease)
        .await
        .unwrap();
    assert!(acked.work_started_at.is_some());
    (held.id, lease)
}

async fn wall(store: &dyn Store, thread: ThreadId) -> i64 {
    store
        .get_thread_budget(thread)
        .await
        .unwrap()
        .map_or(0, |b| b.used_wall_secs)
}

fn audit(workspace: WorkspaceId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(workspace),
        actor_id: None,
        action: action.into(),
        target_kind: Some("member".into()),
        target_id: None,
        metadata: serde_json::json!({}),
    }
}

async fn every_ending_charges_the_time_the_claim_worked(store: &dyn Store) {
    let r = room(store, "ends").await;
    let release_id = task(store, r.channel, "release").await;
    let unassign_id = task(store, r.channel, "unassign").await;
    let reassign_id = task(store, r.channel, "reassign").await;
    let freeze_id = task(store, r.channel, "freeze").await;
    let close_id = task(store, r.channel, "close").await;
    let stop_id = task(store, r.channel, "stop").await;
    let workspace = r.workspace;
    let (scim_member, _) = store
        .scim_provision_audited(
            NewMember {
                workspace_id: workspace,
                handle: "scim".into(),
                display_name: None,
                kind: MemberKind::Agent,
            },
            None,
            true,
            Box::new(move |_| audit(workspace, "scim.user.create")),
        )
        .await
        .unwrap();
    let scim_id = task(store, r.channel, "scim").await;
    let quiet_id = task(store, r.channel, "quiet").await;

    let (release_thread, release_lease) = work(store, r.channel, r.holder).await;
    assert_eq!(release_thread, release_id);
    let (unassign_thread, _) = work(store, r.channel, r.holder).await;
    assert_eq!(unassign_thread, unassign_id);
    let (reassign_thread, _) = work(store, r.channel, r.holder).await;
    assert_eq!(reassign_thread, reassign_id);
    let frozen = store
        .create_member(NewMember {
            workspace_id: r.workspace,
            handle: "frozen".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let (freeze_thread, _) = work(store, r.channel, frozen.id).await;
    assert_eq!(freeze_thread, freeze_id);
    let (close_thread, _) = work(store, r.channel, r.holder).await;
    assert_eq!(close_thread, close_id);
    let (stop_thread, _) = work(store, r.channel, r.holder).await;
    assert_eq!(stop_thread, stop_id);
    let (scim_thread, _) = work(store, r.channel, scim_member.id).await;
    assert_eq!(scim_thread, scim_id);
    // Claimed, never acknowledged: no working clock. Created last, so it is
    // what remains after the acknowledged claims above.
    let quiet = store
        .claim_next_thread(r.channel, r.holder, Some(3600))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(quiet.id, quiet_id);
    assert_eq!(quiet.work_started_at, None);

    // One wait, so every acknowledged claim has a measurable worked time
    // without a sleep per ending. The charge is the database's own interval
    // on Postgres and the host clock on SQLite, both of which stamp
    // `work_started_at`.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    store
        .release_claim_with_event(release_id, r.holder, release_lease)
        .await
        .unwrap();
    store
        .unassign_thread_with_event(unassign_id, r.other)
        .await
        .unwrap();
    store
        .assign_thread_with_event(reassign_id, r.other, r.other, None)
        .await
        .unwrap();
    let (_freeze, released, _) = store
        .freeze_member(frozen.id, r.other, Some("stop"))
        .await
        .unwrap();
    assert_eq!(released, 1, "the freeze releases the one claim");
    store
        .transition_thread(close_id, r.other, ThreadAction::StartReview)
        .await
        .unwrap();
    store
        .transition_thread(close_id, r.other, ThreadAction::Close)
        .await
        .unwrap();
    store
        .set_thread_budget(
            stop_id,
            BudgetLimits {
                max_tokens: Some(1),
                ..BudgetLimits::default()
            },
        )
        .await
        .unwrap();
    let (report, stored) = store
        .report_thread_usage(
            stop_id,
            UsageDelta {
                tokens: 5,
                ..UsageDelta::default()
            },
        )
        .await
        .unwrap();
    assert!(report.stopped, "the token cap stops the run");
    assert_eq!(report.reason.as_deref(), Some("tokens"));
    assert_eq!(stored.map(|e| e.kind), Some(EventKind::ClaimFailed));
    assert!(
        report.budget.used_wall_secs >= 1,
        "the stop keeps the wall time it measured: {}",
        report.budget.used_wall_secs
    );
    store
        .scim_update_user_audited(
            r.workspace,
            scim_member.id,
            None,
            None,
            false,
            audit(r.workspace, "scim.user.deactivate"),
        )
        .await
        .unwrap()
        .unwrap();
    store
        .release_claim(quiet_id, r.holder, quiet.claim_lease_id.unwrap())
        .await
        .unwrap();

    for (id, name) in [
        (release_id, "release"),
        (unassign_id, "unassign"),
        (reassign_id, "reassign"),
        (freeze_id, "freeze"),
        (close_id, "close"),
        (stop_id, "stop"),
        (scim_id, "scim"),
    ] {
        let charged = wall(store, id).await;
        assert!(charged >= 1, "{name} charged {charged}s");
        assert!(
            charged < 30,
            "{name} charged {charged}s, not a second claim"
        );
    }
    assert_eq!(wall(store, quiet_id).await, 0, "never acknowledged");

    for id in [
        release_id,
        unassign_id,
        freeze_id,
        close_id,
        stop_id,
        scim_id,
        quiet_id,
    ] {
        let thread = store.get_thread(id).await.unwrap();
        assert_eq!(thread.assignee_id, None, "{id:?} still held");
        assert_eq!(thread.work_started_at, None);
    }
    let reassigned = store.get_thread(reassign_id).await.unwrap();
    assert_eq!(reassigned.assignee_id, Some(r.other));
    assert_eq!(
        reassigned.work_started_at, None,
        "the new claim has not started"
    );
    assert_eq!(
        store.get_thread(close_id).await.unwrap().state,
        maidan_types::ThreadState::Closed
    );

    // Other threads in this channel are free and under no cap. The stopped
    // one is over its token budget, so it is skipped until the cap is raised.
    let mut handed = Vec::new();
    while let Some(t) = store
        .claim_next_thread(r.channel, r.other, Some(60))
        .await
        .unwrap()
    {
        handed.push(t.id);
    }
    assert!(
        !handed.contains(&stop_id),
        "over budget was handed out: {handed:?}"
    );
    assert!(!handed.is_empty(), "the freed threads are still claimable");
    store
        .patch_thread_budget(
            stop_id,
            BudgetPatch {
                max_tokens: Some(Some(100)),
                ..BudgetPatch::default()
            },
        )
        .await
        .unwrap();
    let raised = store
        .claim_next_thread(r.channel, r.holder, Some(60))
        .await
        .unwrap()
        .expect("raising the token cap puts the stopped thread back");
    assert_eq!(raised.id, stop_id);
}

async fn claim_next_refuses_a_thread_already_over_budget(store: &dyn Store) {
    let r = room(store, "refuse").await;
    let id = task(store, r.channel, "over-tokens").await;
    store
        .set_thread_budget(
            id,
            BudgetLimits {
                max_tokens: Some(1),
                max_usd_micros: Some(1),
                ..BudgetLimits::default()
            },
        )
        .await
        .unwrap();
    // No claim, so the report records the spend and does not stop a run.
    let (report, stored) = store
        .report_thread_usage(
            id,
            UsageDelta {
                tokens: 5,
                usd_micros: 5,
                ..UsageDelta::default()
            },
        )
        .await
        .unwrap();
    assert!(!report.stopped);
    assert!(stored.is_none());
    assert!(
        store
            .claim_next_thread(r.channel, r.holder, Some(60))
            .await
            .unwrap()
            .is_none(),
        "over tokens and usd"
    );
    store
        .patch_thread_budget(
            id,
            BudgetPatch {
                max_tokens: Some(None),
                max_usd_micros: Some(None),
                ..BudgetPatch::default()
            },
        )
        .await
        .unwrap();
    let taken = store
        .claim_next_thread(r.channel, r.other, Some(60))
        .await
        .unwrap()
        .expect("cleared caps are claimable");
    assert_eq!(taken.id, id);
}

async fn run_suite(store: Arc<dyn Store>) {
    claim_next_refuses_a_thread_already_over_budget(store.as_ref()).await;
    every_ending_charges_the_time_the_claim_worked(store.as_ref()).await;
}

#[tokio::test]
async fn claim_wall_every_end_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(Arc::new(SqliteStore::for_tests(pool))).await;
}

#[tokio::test]
async fn claim_wall_every_end_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(Arc::new(PostgresStore::for_tests(pool))).await;
}
