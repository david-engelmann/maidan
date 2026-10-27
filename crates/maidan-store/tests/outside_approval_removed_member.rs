//! A removed member's approval no longer lets a past holder close.
//!
//! The outside-approval rule (#1651) lets a past holder land an owned thread
//! once an outside reviewer approves. A reviewer who is removed afterwards
//! (tombstoned) no longer stands as that outside review (#1664). Two tenants,
//! both backends: tenant A's removed reviewer stops counting while tenant B's
//! live reviewer still lets its own past holder close.

use maidan_fsm::ThreadAction;
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Member, MemberId, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision,
    Thread, WorkspaceId,
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

async fn a_removed_reviewers_approval_no_longer_lets_a_past_holder_close(
    pool: &Pool,
    store: &dyn Store,
) {
    let ws_a = workspace(store, "a").await;
    let ws_b = workspace(store, "b").await;

    let a_owner = member(store, ws_a, "owner").await;
    let a_worker = member(store, ws_a, "worker").await;
    let a_reviewer = member(store, ws_a, "reviewer").await;
    let a_thread = released_thread(store, ws_a, &a_owner, &a_worker).await;
    approve(store, &a_thread, &a_reviewer).await;

    let b_owner = member(store, ws_b, "owner").await;
    let b_worker = member(store, ws_b, "worker").await;
    let b_reviewer = member(store, ws_b, "reviewer").await;
    let b_thread = released_thread(store, ws_b, &b_owner, &b_worker).await;
    approve(store, &b_thread, &b_reviewer).await;

    pool.tombstone(a_reviewer.id).await;

    let refused = store
        .transition_thread(a_thread.id, a_worker.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "a removed reviewer's approval let tenant A's past holder close: {refused:?}"
    );
    store
        .transition_thread(b_thread.id, b_worker.id, ThreadAction::Close)
        .await
        .expect("tenant B's live reviewer still lets its past holder close");
}

#[tokio::test]
async fn a_removed_reviewers_approval_no_longer_lets_a_past_holder_close_sqlite() {
    let (pool, store) = sqlite().await;
    a_removed_reviewers_approval_no_longer_lets_a_past_holder_close(&pool, &store).await;
}

#[tokio::test]
async fn a_removed_reviewers_approval_no_longer_lets_a_past_holder_close_postgres() {
    let Some((_c, pool, store)) = postgres().await else {
        return;
    };
    a_removed_reviewers_approval_no_longer_lets_a_past_holder_close(&pool, &store).await;
}
