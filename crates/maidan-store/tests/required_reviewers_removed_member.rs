//! A removed member's approval no longer counts toward a thread's required
//! reviewers (#1684, follow-up to #1664).
//!
//! The review gate counted every approval on the thread, so an approval from a
//! reviewer — or submitted by a delegate — who has since left the workspace
//! still met the requirement. Two tenants, both backends: tenant B's live
//! members are the control that still closes.

use maidan_fsm::ThreadAction;
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Member, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision,
    Thread, ThreadId, WorkspaceId,
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

/// Opens a thread in `ws` that needs one approval, approved by `reviewer`
/// (submitted by `submitter` when one is given).
async fn approved_thread(
    pool: &Pool,
    store: &dyn Store,
    ws: WorkspaceId,
    tag: &str,
    submitter: Option<&Member>,
) -> (Member, Member, Thread) {
    let owner = member(store, ws, &format!("owner-{tag}")).await;
    let worker = member(store, ws, &format!("worker-{tag}")).await;
    let reviewer = member(store, ws, &format!("reviewer-{tag}")).await;
    let t = released_thread(store, ws, &owner, &worker).await;
    store
        .set_review_requirement(t.id, 1)
        .await
        .expect("requirement");
    approve(store, &t, &reviewer).await;
    if let Some(s) = submitter {
        pool.set_submitter(t.id, reviewer.id, s.id).await;
    }
    (owner, reviewer, t)
}

async fn a_removed_reviewers_approval_no_longer_meets_the_requirement(
    pool: &Pool,
    store: &dyn Store,
) {
    let ws_a = workspace(store, "a").await;
    let ws_b = workspace(store, "b").await;
    let (owner_a, reviewer_a, a) = approved_thread(pool, store, ws_a, "a", None).await;
    let (owner_b, _, b) = approved_thread(pool, store, ws_b, "b", None).await;
    pool.tombstone(reviewer_a.id).await;
    let refused = store
        .transition_thread(a.id, owner_a.id, ThreadAction::Close)
        .await;
    assert!(
        refused.is_err(),
        "a removed reviewer's approval met tenant A's requirement: {refused:?}"
    );
    store
        .transition_thread(b.id, owner_b.id, ThreadAction::Close)
        .await
        .expect("tenant B's live reviewer still meets its requirement");
}

async fn an_approval_submitted_by_a_removed_or_foreign_member_does_not_meet_the_requirement(
    pool: &Pool,
    store: &dyn Store,
) {
    // Tenant A runs each case in its own workspace, so the threads do not
    // collide; tenant B is the control.
    let ws_a = workspace(store, "a").await;
    let ws_a2 = workspace(store, "a2").await;
    let ws_b = workspace(store, "b").await;
    let removed = member(store, ws_a, "removed").await;
    let stranger = member(store, ws_b, "stranger").await;
    let live = member(store, ws_b, "live").await;
    let (owner_r, _, by_removed) =
        approved_thread(pool, store, ws_a, "removed", Some(&removed)).await;
    let (owner_f, _, by_stranger) =
        approved_thread(pool, store, ws_a2, "foreign", Some(&stranger)).await;
    let (owner_b, _, b) = approved_thread(pool, store, ws_b, "b", Some(&live)).await;
    pool.tombstone(removed.id).await;
    let refused = store
        .transition_thread(by_removed.id, owner_r.id, ThreadAction::Close)
        .await;
    assert!(
        refused.is_err(),
        "an approval submitted by a removed member met tenant A's requirement: {refused:?}"
    );
    let refused = store
        .transition_thread(by_stranger.id, owner_f.id, ThreadAction::Close)
        .await;
    assert!(
        refused.is_err(),
        "an approval submitted by tenant B's member met tenant A's requirement: {refused:?}"
    );
    store
        .transition_thread(b.id, owner_b.id, ThreadAction::Close)
        .await
        .expect("tenant B's approval submitted by its own live member still meets its requirement");
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
    a_removed_reviewers_approval_no_longer_meets_the_requirement,
    a_removed_reviewers_approval_no_longer_meets_the_requirement_sqlite,
    a_removed_reviewers_approval_no_longer_meets_the_requirement_postgres
);
both_backends!(
    an_approval_submitted_by_a_removed_or_foreign_member_does_not_meet_the_requirement,
    an_approval_submitted_by_a_removed_or_foreign_member_does_not_meet_the_requirement_sqlite,
    an_approval_submitted_by_a_removed_or_foreign_member_does_not_meet_the_requirement_postgres
);
