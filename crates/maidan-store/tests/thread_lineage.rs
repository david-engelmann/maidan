//! Run lineage (Cluster 387.1, Wave 2 #28): a thread homes a producer's
//! `run_id` as `parent_run_id`. Nested occupancy attributes every open
//! thread that shares the value. F7 mute is orthogonal — a muted nested
//! thread still counts. Both backends. Does not mint a parallel id.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    normalize_parent_run_id, run_id_from_payload, MemberKind, NewChannel, NewMember, NewThread,
    NewWorkspace, PARENT_RUN_ID_MAX_BYTES,
};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

/// The authoritative waiter fixture's `run_id` — the first real producer.
const PI_RUN_ID: &str = "aa4dc966-0e09-44c3-b7a5-2d048b48b301";

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
    SqliteStore::new(pool)
}

async fn seed(
    store: &dyn Store,
) -> (
    maidan_types::WorkspaceId,
    maidan_types::MemberId,
    Vec<maidan_types::Thread>,
) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "lineage".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let parent = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parent run".into()),
        })
        .await
        .expect("parent");
    let child = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: Some(parent.id),
            title: Some("nested".into()),
        })
        .await
        .expect("child");
    let other = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("unrelated".into()),
        })
        .await
        .expect("other");
    (ws.id, member.id, vec![parent, child, other])
}

async fn run_suite(store: &dyn Store) {
    let (ws, member, threads) = seed(store).await;
    let parent = &threads[0];
    let child = &threads[1];
    let other = &threads[2];

    assert!(store
        .get_thread_lineage(parent.id)
        .await
        .expect("get empty")
        .is_none());

    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../maidan-types/tests/fixtures/waiter_result_v1.json"
    ))
    .expect("fixture");
    let extracted = run_id_from_payload(&fixture).expect("fixture run_id");
    assert_eq!(extracted, PI_RUN_ID);

    let lined = store
        .set_thread_lineage(parent.id, extracted)
        .await
        .expect("home parent");
    assert_eq!(lined.parent_run_id, PI_RUN_ID);
    assert_eq!(lined.thread_id, parent.id);

    let child_lined = store
        .set_thread_lineage(child.id, &format!("  {PI_RUN_ID}  "))
        .await
        .expect("trim accepted");
    assert_eq!(
        child_lined.parent_run_id, PI_RUN_ID,
        "the stored value is the trimmed producer id, not a minted uuid"
    );

    store
        .set_thread_lineage(other.id, "some-other-producer-run")
        .await
        .expect("other run");

    let listed = store
        .list_threads_for_run(ws, PI_RUN_ID)
        .await
        .expect("list");
    let ids: Vec<_> = listed.iter().map(|t| t.id).collect();
    assert_eq!(
        ids,
        vec![parent.id, child.id],
        "nested children share the run"
    );
    assert!(
        !ids.contains(&other.id),
        "a different producer run is not attributed"
    );

    let occ0 = store.run_occupancy(ws, PI_RUN_ID).await.expect("occ0");
    assert_eq!(occ0.parent_run_id, PI_RUN_ID);
    assert_eq!(occ0.open, 2, "parent + nested child");
    assert_eq!(occ0.queued, 2);
    assert_eq!(occ0.claimed, 0);
    assert_eq!(occ0.working, 0);
    assert_eq!(occ0.blocked, 0);

    let claimed = store
        .claim_thread(child.id, member)
        .await
        .expect("claim nested");
    assert!(claimed.claimed);
    let lease = claimed
        .thread
        .claim_lease_id
        .expect("a claim mints a lease");
    let occ_claimed = store.run_occupancy(ws, PI_RUN_ID).await.expect("claimed");
    assert_eq!(occ_claimed.queued, 1);
    assert_eq!(occ_claimed.claimed, 1);
    assert_eq!(occ_claimed.working, 0);

    store
        .acknowledge_claim(child.id, member, lease)
        .await
        .expect("ack");
    let occ_working = store.run_occupancy(ws, PI_RUN_ID).await.expect("working");
    assert_eq!(occ_working.queued, 1);
    assert_eq!(occ_working.claimed, 0);
    assert_eq!(occ_working.working, 1);
    assert_eq!(occ_working.open, 2);

    store
        .mute_thread(member, child.id)
        .await
        .expect("F7 mute the nested thread");
    let occ_muted = store.run_occupancy(ws, PI_RUN_ID).await.expect("muted");
    assert_eq!(
        occ_muted, occ_working,
        "F7 mute is orthogonal — muted nested work still counts"
    );
    assert!(
        store
            .is_thread_muted(member, child.id)
            .await
            .expect("muted?"),
        "the mute row exists; occupancy just does not consult it"
    );

    let empty = store
        .run_occupancy(ws, "no-such-run")
        .await
        .expect("unknown run");
    assert_eq!(empty.open, 0);
    assert!(store
        .list_threads_for_run(ws, "no-such-run")
        .await
        .expect("empty list")
        .is_empty());

    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .expect("other ws");
    let isolated = store
        .list_threads_for_run(other_ws.id, PI_RUN_ID)
        .await
        .expect("cross-tenant");
    assert!(isolated.is_empty(), "lineage is workspace-scoped");

    assert!(matches!(
        store.set_thread_lineage(parent.id, "   ").await,
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store
            .set_thread_lineage(parent.id, &"x".repeat(PARENT_RUN_ID_MAX_BYTES + 1))
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    assert!(normalize_parent_run_id("").is_none());
    assert!(run_id_from_payload(&json!({"run_id": "  "})).is_none());
    assert!(run_id_from_payload(&json!({"run_id": 12})).is_none());

    assert!(store.clear_thread_lineage(other.id).await.expect("clear"));
    assert!(!store
        .clear_thread_lineage(other.id)
        .await
        .expect("idempotent"));
    assert!(store
        .get_thread_lineage(other.id)
        .await
        .expect("cleared")
        .is_none());

    let rewritten = store
        .set_thread_lineage(parent.id, "rewritten-by-producer")
        .await
        .expect("overwrite");
    assert_eq!(rewritten.parent_run_id, "rewritten-by-producer");
}

#[tokio::test]
async fn run_lineage_homes_producer_run_id_and_attributes_nested_occupancy_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn run_lineage_homes_producer_run_id_and_attributes_nested_occupancy_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
