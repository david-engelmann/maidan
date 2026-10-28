//! Closing a thread that will not land (`close_abandoned_thread`): who may
//! do it, what `duplicate_of` may name, the reference and audit row it
//! leaves, and that none of it crosses into another workspace, identically
//! on both backends.

use std::time::Duration;

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

async fn member(store: &dyn Store, workspace_id: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn channel(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    name: &str,
    private: bool,
) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .unwrap()
        .id
}

async fn task(store: &dyn Store, channel_id: ChannelId, by: MemberId, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: Some(by),
        })
        .await
        .unwrap()
        .id
}

fn audit(ws: WorkspaceId, actor: MemberId, thread: ThreadId) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: Some(actor),
        action: "thread.close_abandoned".into(),
        target_kind: Some("thread".into()),
        target_id: Some(thread.0),
        metadata: serde_json::json!({}),
    }
}

fn close(reason: CloseReason, duplicate_of: Option<ThreadId>) -> CloseAbandoned {
    CloseAbandoned {
        reason,
        duplicate_of,
    }
}

async fn assert_close_abandoned(store: &dyn Store) {
    let ws_a = workspace(store, "close-a").await;
    let ws_b = workspace(store, "close-b").await;
    let alice = member(store, ws_a, "alice").await;
    let carol = member(store, ws_a, "carol").await;
    let mallory = member(store, ws_a, "mallory").await;
    let bob = member(store, ws_b, "bob").await;
    let open_a = channel(store, ws_a, "open-a", false).await;
    let private_a = channel(store, ws_a, "private-a", true).await;
    let open_b = channel(store, ws_b, "open-b", false).await;

    // The producer closes its own thread with no result: no review gate.
    let t_gone = task(store, open_a, alice, "nobody will do this").await;
    let (result, _) = store
        .close_abandoned_thread(
            t_gone,
            alice,
            false,
            &close(CloseReason::Abandoned, None),
            audit(ws_a, alice, t_gone),
        )
        .await
        .unwrap();
    assert_eq!(result.from_state, ThreadState::Open);
    assert_eq!(result.thread.state, ThreadState::Closed);

    // The audit row is in the thread's workspace only, with its reason.
    let rows = store.list_audit_for_workspace(ws_a, 50).await.unwrap();
    let row = rows
        .iter()
        .find(|r| r.action == "thread.close_abandoned" && r.target_id == Some(t_gone.0))
        .expect("the close is audited in its workspace");
    assert_eq!(row.metadata["reason"], "abandoned");
    assert_eq!(row.metadata["had_result"], false);
    assert_eq!(row.metadata["from_state"], "open");
    let other = store.list_audit_for_workspace(ws_b, 50).await.unwrap();
    assert!(other.iter().all(|r| r.target_id != Some(t_gone.0)));

    // An already-closed thread cannot be closed again this way.
    let again = store
        .close_abandoned_thread(
            t_gone,
            alice,
            false,
            &close(CloseReason::Abandoned, None),
            audit(ws_a, alice, t_gone),
        )
        .await;
    assert!(matches!(again, Err(StoreError::Conflict(_))), "{again:?}");

    // Someone who neither produced nor holds the thread is refused.
    let t_kept = task(store, open_a, alice, "still wanted").await;
    let stranger = store
        .close_abandoned_thread(
            t_kept,
            mallory,
            false,
            &close(CloseReason::Abandoned, None),
            audit(ws_a, mallory, t_kept),
        )
        .await;
    assert!(
        matches!(stranger, Err(StoreError::Conflict(_))),
        "{stranger:?}"
    );
    assert_eq!(
        store.get_thread(t_kept).await.unwrap().state,
        ThreadState::Open
    );

    // The claim holder may close it.
    let t_held = task(store, open_a, alice, "carol holds this").await;
    assert!(store.claim_thread(t_held, carol).await.unwrap().claimed);
    let (held, _) = store
        .close_abandoned_thread(
            t_held,
            carol,
            false,
            &close(CloseReason::Superseded, None),
            audit(ws_a, carol, t_held),
        )
        .await
        .unwrap();
    assert_eq!(held.thread.state, ThreadState::Closed);
    assert_eq!(held.thread.assignee_id, None);

    // A maintainer may close a thread it neither produced nor holds.
    let (kept, _) = store
        .close_abandoned_thread(
            t_kept,
            mallory,
            true,
            &close(CloseReason::Abandoned, None),
            audit(ws_a, mallory, t_kept),
        )
        .await
        .unwrap();
    assert_eq!(kept.thread.state, ThreadState::Closed);

    // `duplicate_of` must fit the reason and not be the thread itself.
    let t_dup = task(store, open_a, alice, "same as the original").await;
    let t_orig = task(store, open_a, alice, "the original").await;
    for (reason, dup) in [
        (CloseReason::Abandoned, Some(t_orig)),
        (CloseReason::Duplicate, None),
        (CloseReason::Duplicate, Some(t_dup)),
    ] {
        let refused = store
            .close_abandoned_thread(
                t_dup,
                alice,
                false,
                &close(reason, dup),
                audit(ws_a, alice, t_dup),
            )
            .await;
        assert!(
            matches!(refused, Err(StoreError::InvalidInput(_))),
            "{refused:?}"
        );
    }

    // A thread in another workspace, one the closer cannot read, and a
    // made-up id all get the same refusal, and nothing is closed.
    let t_foreign = task(store, open_b, bob, "bob's thread").await;
    let t_private = task(store, private_a, carol, "carol's private thread").await;
    let mut refusals = Vec::new();
    for dup in [t_foreign, t_private, ThreadId(uuid::Uuid::now_v7())] {
        match store
            .close_abandoned_thread(
                t_dup,
                alice,
                false,
                &close(CloseReason::Duplicate, Some(dup)),
                audit(ws_a, alice, t_dup),
            )
            .await
        {
            Err(StoreError::InvalidInput(message)) => refusals.push(message),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }
    assert!(refusals.windows(2).all(|w| w[0] == w[1]), "{refusals:?}");
    assert_eq!(
        store.get_thread(t_dup).await.unwrap().state,
        ThreadState::Open
    );

    // A duplicate records `thread duplicates original`.
    let (dup, events) = store
        .close_abandoned_thread(
            t_dup,
            alice,
            false,
            &close(CloseReason::Duplicate, Some(t_orig)),
            audit(ws_a, alice, t_dup),
        )
        .await
        .unwrap();
    assert_eq!(dup.thread.state, ThreadState::Closed);
    assert!(events.len() >= 2, "a reference and a state change");
    let refs = store
        .list_references_from(RefSide::Thread, t_dup.0)
        .await
        .unwrap();
    assert!(refs
        .iter()
        .any(|r| r.dst_id == t_orig.0 && r.relation == RelationKind::Duplicates));

    // A superseded thread records `replacement supersedes thread`.
    let t_old = task(store, open_a, alice, "the old plan").await;
    let t_new = task(store, open_a, alice, "the new plan").await;
    store
        .close_abandoned_thread(
            t_old,
            alice,
            false,
            &close(CloseReason::Superseded, Some(t_new)),
            audit(ws_a, alice, t_old),
        )
        .await
        .unwrap();
    let refs = store
        .list_references_from(RefSide::Thread, t_new.0)
        .await
        .unwrap();
    assert!(refs
        .iter()
        .any(|r| r.dst_id == t_old.0 && r.relation == RelationKind::Supersedes));

    // Bob's thread in the other workspace is untouched throughout.
    assert_eq!(
        store.get_thread(t_foreign).await.unwrap().state,
        ThreadState::Open
    );
}

#[tokio::test]
async fn only_its_producer_holder_or_a_maintainer_closes_an_abandoned_thread_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool);
    assert_close_abandoned(&store).await;
}

#[tokio::test]
async fn only_its_producer_holder_or_a_maintainer_closes_an_abandoned_thread_on_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::for_tests(pool);
    assert_close_abandoned(&store).await;
}
