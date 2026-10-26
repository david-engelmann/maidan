//! A past holder of an owned thread may land it only after an outside review.
//!
//! The past-holder rule (#1569) stops someone who once held the claim from
//! standing in for an independent review. An approval from an outside reviewer
//! (not the owner, never a holder, in the thread's own workspace) still lets a
//! past holder close; without one the close is refused. Both backends.

use maidan_fsm::ThreadAction;
use maidan_store::test_support::SetResultClaiming;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Member, MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ReviewDecision, Thread,
    WorkspaceId,
};
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::for_tests(pool)
}

async fn postgres() -> Option<(ContainerAsync<Postgres>, PostgresStore)> {
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
    Some((container, PostgresStore::for_tests(pool)))
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

async fn past_holder_cannot_close_without_an_outside_approval(store: &dyn Store) {
    let ws = workspace(store, "a").await;
    let owner = member(store, ws, "owner").await;
    let worker = member(store, ws, "worker").await;
    let t = released_thread(store, ws, &owner, &worker).await;
    let refused = store
        .transition_thread(t.id, worker.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "a past holder closed with no outside approval: {refused:?}"
    );
}

async fn past_holder_can_close_after_an_outside_approval(store: &dyn Store) {
    let ws = workspace(store, "a").await;
    let owner = member(store, ws, "owner").await;
    let worker = member(store, ws, "worker").await;
    let reviewer = member(store, ws, "reviewer").await;
    let t = released_thread(store, ws, &owner, &worker).await;
    approve(store, &t, &reviewer).await;
    store
        .transition_thread(t.id, worker.id, ThreadAction::Close)
        .await
        .expect("an outside approval lets a past holder close");
}

async fn an_approval_from_another_tenant_does_not_let_a_past_holder_close(store: &dyn Store) {
    let ws_a = workspace(store, "a").await;
    let ws_b = workspace(store, "b").await;
    let owner = member(store, ws_a, "owner").await;
    let worker = member(store, ws_a, "worker").await;
    let stranger = member(store, ws_b, "stranger").await;
    let t = released_thread(store, ws_a, &owner, &worker).await;
    // The store may refuse the cross-tenant review outright; either way it
    // must not count.
    let root = store
        .latest_review_packet(t.id)
        .await
        .expect("packet")
        .map(|p| p.evidence_root);
    let _ = store
        .submit_review(
            t.id,
            stranger.id,
            ReviewDecision::Approve,
            None,
            root.as_deref(),
        )
        .await;
    let refused = store
        .transition_thread(t.id, worker.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "tenant B's approval let tenant A's past holder close: {refused:?}"
    );
}

#[tokio::test]
async fn a_past_holder_cannot_close_without_an_outside_approval_sqlite() {
    past_holder_cannot_close_without_an_outside_approval(&sqlite().await).await;
}

#[tokio::test]
async fn a_past_holder_cannot_close_without_an_outside_approval_postgres() {
    let Some((_c, store)) = postgres().await else {
        return;
    };
    past_holder_cannot_close_without_an_outside_approval(&store).await;
}

#[tokio::test]
async fn a_past_holder_can_close_after_an_outside_approval_sqlite() {
    past_holder_can_close_after_an_outside_approval(&sqlite().await).await;
}

#[tokio::test]
async fn a_past_holder_can_close_after_an_outside_approval_postgres() {
    let Some((_c, store)) = postgres().await else {
        return;
    };
    past_holder_can_close_after_an_outside_approval(&store).await;
}

#[tokio::test]
async fn an_approval_from_another_tenant_does_not_let_a_past_holder_close_sqlite() {
    an_approval_from_another_tenant_does_not_let_a_past_holder_close(&sqlite().await).await;
}

#[tokio::test]
async fn an_approval_from_another_tenant_does_not_let_a_past_holder_close_postgres() {
    let Some((_c, store)) = postgres().await else {
        return;
    };
    an_approval_from_another_tenant_does_not_let_a_past_holder_close(&store).await;
}

/// Two holders take turns on one owned thread, then it goes to review: each
/// is a past holder, so neither stands as the other's outside reviewer.
async fn two_holders_cannot_approve_each_other_and_another_tenants_approval_does_not_carry(
    store: &dyn Store,
) {
    let ws_a = workspace(store, "a").await;
    let ws_b = workspace(store, "b").await;
    let owner = member(store, ws_a, "owner").await;
    let first = member(store, ws_a, "first").await;
    let second = member(store, ws_a, "second").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws_a,
            name: "pair".into(),
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
    for holder in [&first, &second] {
        store
            .set_thread_result_claiming(t.id, holder.id, &serde_json::json!({"status": "done"}))
            .await
            .expect("result");
        store.unassign_thread(t.id).await.expect("release");
    }
    store
        .transition_thread(t.id, owner.id, ThreadAction::StartReview)
        .await
        .expect("start review");

    // Each past holder approves; neither approval is outside.
    approve(store, &t, &second).await;
    approve(store, &t, &first).await;
    for holder in [&first, &second] {
        let refused = store
            .transition_thread(t.id, holder.id, ThreadAction::Close)
            .await;
        assert!(
            matches!(refused, Err(StoreError::Conflict(_))),
            "a fellow past holder's approval let {} close: {refused:?}",
            holder.handle
        );
    }

    // Tenant B has its own owned thread with a genuine outside approval;
    // it does not carry over to tenant A's thread.
    let b_owner = member(store, ws_b, "owner").await;
    let b_worker = member(store, ws_b, "worker").await;
    let b_reviewer = member(store, ws_b, "reviewer").await;
    let b_thread = released_thread(store, ws_b, &b_owner, &b_worker).await;
    approve(store, &b_thread, &b_reviewer).await;
    let refused = store
        .transition_thread(t.id, first.id, ThreadAction::Close)
        .await;
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "tenant B's approval on its own thread let tenant A's past holder close: {refused:?}"
    );
    store
        .transition_thread(b_thread.id, b_worker.id, ThreadAction::Close)
        .await
        .expect("tenant B's past holder closes after its own outside approval");
}

#[tokio::test]
async fn two_past_holders_cannot_approve_each_other_and_another_tenants_approval_does_not_carry_sqlite(
) {
    two_holders_cannot_approve_each_other_and_another_tenants_approval_does_not_carry(
        &sqlite().await,
    )
    .await;
}

#[tokio::test]
async fn two_past_holders_cannot_approve_each_other_and_another_tenants_approval_does_not_carry_postgres(
) {
    let Some((_c, store)) = postgres().await else {
        return;
    };
    two_holders_cannot_approve_each_other_and_another_tenants_approval_does_not_carry(&store).await;
}
