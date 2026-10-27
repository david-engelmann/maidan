//! Every member an outside approval leans on must still belong to the thread's
//! workspace (#1685, follow-up to #1664).
//!
//! An outside approval names up to three members: the reviewer, the member who
//! submitted the review for them (`actor_id`), and, for a land-gate pass, the
//! recorder. Each must be live and in the thread's own workspace, or the
//! approval no longer lets a past holder close. Two tenants, both backends:
//! tenant B's live members are the control that still closes.

use maidan_fsm::ThreadAction;
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    LandGateStatus, Member, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace,
    ReviewDecision, Thread, ThreadId, WorkspaceId, LAND_GATE_SKILL,
};
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;

/// The raw pool, so the test can remove a member the way SCIM does.
enum Pool {
    Sqlite(sqlx::SqlitePool),
    Postgres(sqlx::PgPool),
}

impl Pool {
    async fn tombstone(&self, id: MemberId) {
        let now = chrono::Utc::now();
        match self {
            Pool::Sqlite(p) => {
                sqlx::query("UPDATE maidan_members SET tombstoned_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(id.0)
                    .execute(p)
                    .await
                    .expect("tombstone");
            }
            Pool::Postgres(p) => {
                sqlx::query("UPDATE maidan_members SET tombstoned_at = $1 WHERE id = $2")
                    .bind(now)
                    .bind(id.0)
                    .execute(p)
                    .await
                    .expect("tombstone");
            }
        }
    }

    /// Records that `actor` submitted `reviewer`'s review, as a delegate's
    /// request does.
    async fn set_submitter(&self, thread: ThreadId, reviewer: MemberId, actor: MemberId) {
        match self {
            Pool::Sqlite(p) => {
                sqlx::query(
                    "UPDATE maidan_thread_reviews SET actor_id = ? WHERE thread_id = ? AND reviewer_id = ?",
                )
                .bind(actor.0)
                .bind(thread.0)
                .bind(reviewer.0)
                .execute(p)
                .await
                .expect("submitter");
            }
            Pool::Postgres(p) => {
                sqlx::query(
                    "UPDATE maidan_thread_reviews SET actor_id = $1 WHERE thread_id = $2 AND reviewer_id = $3",
                )
                .bind(actor.0)
                .bind(thread.0)
                .bind(reviewer.0)
                .execute(p)
                .await
                .expect("submitter");
            }
        }
    }
}

async fn sqlite() -> (Pool, SqliteStore) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    (Pool::Sqlite(pool.clone()), SqliteStore::for_tests(pool))
}

async fn postgres() -> Option<(ContainerAsync<Postgres>, Pool, PostgresStore)> {
    use maidan_store::run_postgres_migrations;
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    Some((
        container,
        Pool::Postgres(pool.clone()),
        PostgresStore::for_tests(pool),
    ))
}

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws")
        .id
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> Member {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect(handle)
}

/// An owned thread the worker held, posted a result on, released and handed
/// to review, so the worker is now a past holder.
async fn released_thread(
    store: &dyn Store,
    ws: WorkspaceId,
    owner: &Member,
    worker: &Member,
) -> Thread {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "ph".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let t = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    store
        .set_thread_owner(t.id, Some(owner.id))
        .await
        .expect("owner");
    store
        .set_thread_result_claiming(t.id, worker.id, &serde_json::json!({"status": "done"}))
        .await
        .expect("result");
    store.unassign_thread(t.id).await.expect("release");
    store
        .transition_thread(t.id, owner.id, ThreadAction::StartReview)
        .await
        .expect("start review");
    t
}

async fn approve(store: &dyn Store, t: &Thread, reviewer: &Member) {
    let root = store
        .latest_review_packet(t.id)
        .await
        .expect("packet")
        .map(|p| p.evidence_root);
    store
        .submit_review(
            t.id,
            reviewer.id,
            ReviewDecision::Approve,
            None,
            root.as_deref(),
        )
        .await
        .expect("approve");
}

fn assert_refused(r: Result<impl std::fmt::Debug, StoreError>, why: &str) {
    assert!(matches!(r, Err(StoreError::Conflict(_))), "{why}: {r:?}");
}

/// Tenant B: a live reviewer submitted by a live delegate of its own workspace
/// still lets its past holder close.
async fn tenant_b_control(pool: &Pool, store: &dyn Store, ws_b: WorkspaceId) {
    let owner = member(store, ws_b, "owner").await;
    let worker = member(store, ws_b, "worker").await;
    let reviewer = member(store, ws_b, "reviewer").await;
    let submitter = member(store, ws_b, "submitter").await;
    let t = released_thread(store, ws_b, &owner, &worker).await;
    approve(store, &t, &reviewer).await;
    pool.set_submitter(t.id, reviewer.id, submitter.id).await;
    store
        .transition_thread(t.id, worker.id, ThreadAction::Close)
        .await
        .expect("tenant B's live reviewer and submitter still let its past holder close");
}

async fn a_removed_submitters_review_no_longer_lets_a_past_holder_close(
    pool: &Pool,
    store: &dyn Store,
) {
    let ws_a = workspace(store, "a").await;
    let ws_b = workspace(store, "b").await;
    let owner = member(store, ws_a, "owner").await;
    let worker = member(store, ws_a, "worker").await;
    let reviewer = member(store, ws_a, "reviewer").await;
    let submitter = member(store, ws_a, "submitter").await;
    let t = released_thread(store, ws_a, &owner, &worker).await;
    approve(store, &t, &reviewer).await;
    pool.set_submitter(t.id, reviewer.id, submitter.id).await;
    pool.tombstone(submitter.id).await;
    assert_refused(
        store
            .transition_thread(t.id, worker.id, ThreadAction::Close)
            .await,
        "a review submitted by a removed member let tenant A's past holder close",
    );
    tenant_b_control(pool, store, ws_b).await;
}

async fn a_review_submitted_from_another_workspace_does_not_let_a_past_holder_close(
    pool: &Pool,
    store: &dyn Store,
) {
    let ws_a = workspace(store, "a").await;
    let ws_b = workspace(store, "b").await;
    let owner = member(store, ws_a, "owner").await;
    let worker = member(store, ws_a, "worker").await;
    let reviewer = member(store, ws_a, "reviewer").await;
    let stranger = member(store, ws_b, "stranger").await;
    let t = released_thread(store, ws_a, &owner, &worker).await;
    approve(store, &t, &reviewer).await;
    pool.set_submitter(t.id, reviewer.id, stranger.id).await;
    assert_refused(
        store
            .transition_thread(t.id, worker.id, ThreadAction::Close)
            .await,
        "a review submitted by tenant B's member let tenant A's past holder close",
    );
    tenant_b_control(pool, store, ws_b).await;
}

async fn a_removed_recorders_land_gate_pass_no_longer_lets_a_past_holder_close(
    pool: &Pool,
    store: &dyn Store,
) {
    let mut closes = Vec::new();
    for name in ["a", "b"] {
        let ws = workspace(store, name).await;
        let owner = member(store, ws, "owner").await;
        let worker = member(store, ws, "worker").await;
        let recorder = member(store, ws, "recorder").await;
        store
            .add_member_skill(recorder.id, LAND_GATE_SKILL)
            .await
            .expect("skill");
        let t = released_thread(store, ws, &owner, &worker).await;
        store
            .set_land_gate_pointer(t.id, recorder.id, LandGateStatus::Pass, None, None)
            .await
            .expect("pass");
        // Tenant A's recorder is removed; tenant B's stays.
        if name == "a" {
            pool.tombstone(recorder.id).await;
        }
        closes.push(
            store
                .transition_thread(t.id, worker.id, ThreadAction::Close)
                .await,
        );
    }
    let b = closes.pop().expect("b");
    let a = closes.pop().expect("a");
    assert_refused(
        a,
        "a removed recorder's land-gate pass let tenant A's past holder close",
    );
    b.expect("tenant B's live recorder's pass still lets its past holder close");
}

macro_rules! both_backends {
    ($scenario:ident, $sqlite:ident, $postgres:ident) => {
        #[tokio::test]
        async fn $sqlite() {
            let (pool, store) = sqlite().await;
            $scenario(&pool, &store).await;
        }

        #[tokio::test]
        async fn $postgres() {
            let Some((_c, pool, store)) = postgres().await else {
                return;
            };
            $scenario(&pool, &store).await;
        }
    };
}

both_backends!(
    a_removed_submitters_review_no_longer_lets_a_past_holder_close,
    a_removed_submitters_review_no_longer_lets_a_past_holder_close_sqlite,
    a_removed_submitters_review_no_longer_lets_a_past_holder_close_postgres
);
both_backends!(
    a_review_submitted_from_another_workspace_does_not_let_a_past_holder_close,
    a_review_submitted_from_another_workspace_does_not_let_a_past_holder_close_sqlite,
    a_review_submitted_from_another_workspace_does_not_let_a_past_holder_close_postgres
);
both_backends!(
    a_removed_recorders_land_gate_pass_no_longer_lets_a_past_holder_close,
    a_removed_recorders_land_gate_pass_no_longer_lets_a_past_holder_close_sqlite,
    a_removed_recorders_land_gate_pass_no_longer_lets_a_past_holder_close_postgres
);
