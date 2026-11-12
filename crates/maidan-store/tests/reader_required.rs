//! Every store listing that filters by a reader takes the reader as a
//! required `MemberId`. There is no `None` that reads as "no filter", the
//! optional-id pattern CLAUDE.md calls an authorization bug. Two tenants on
//! both backends: a member of workspace B reads nothing on workspace A's
//! threads through any of these listings, while A's own member does. And the
//! shared SQL listing filter shows no thread row to a NULL reader bind, so a
//! reader that is somehow unbound hides everything rather than showing it.

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use maidan_store::store::WorkspaceThreadSearch;
use maidan_store::{
    prelude::*, run_postgres_migrations, run_sqlite_migrations, A2aTaskQuery, A2aTaskWrite,
    PendingGateQuery,
};
use maidan_types::*;

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id
}

async fn post(store: &dyn Store, thread: ThreadId, author: MemberId, body: &str) -> Message {
    let (message, _) = store
        .post_message_with_event(
            NewMessage {
                thread_id: thread,
                author_id: author,
                body: body.into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    message
}

fn posted(census: &KindCensus) -> i64 {
    census
        .counts
        .iter()
        .find(|c| c.kind == EventKind::MessagePosted)
        .map(|c| c.count)
        .unwrap_or(0)
}

/// Workspace A with one public thread carrying a message, a pinned target,
/// a tombstone, a pending gate and an A2A task; workspace B with only `bob`.
struct Tenants {
    ws_a: WorkspaceId,
    channel_a: ChannelId,
    thread_a: ThreadId,
    target: MessageId,
    alice: MemberId,
    bob: MemberId,
}

async fn tenants(store: &dyn Store) -> Tenants {
    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap()
        .id;
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let alice = member(store, ws_a, "alice").await;
    let bob = member(store, ws_b, "bob").await;
    let channel_a = store
        .create_channel(NewChannel {
            workspace_id: ws_a,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id;
    let thread_a = store
        .create_thread(NewThread {
            channel_id: channel_a,
            parent_thread_id: None,
            title: Some("public work".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id;
    let target = post(store, thread_a, alice, "target").await;
    store
        .pin_message_with_event(NewPin {
            thread_id: thread_a,
            message_id: target.id,
            member_id: alice,
        })
        .await
        .unwrap();
    let dead = post(store, thread_a, alice, "drop").await;
    store
        .tombstone_message_with_event(dead.id, None)
        .await
        .unwrap();
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws_a,
            thread_id: Some(thread_a),
            requested_by: alice,
            prompt: "ship it?".into(),
            schema: None,
            risk: Default::default(),
        })
        .await
        .unwrap();
    store
        .upsert_a2a_task(A2aTaskWrite {
            workspace_id: ws_a,
            task_id: "task-a",
            context_id: Some("ctx-a"),
            thread_id: Some(thread_a),
            state: "TASK_STATE_WORKING",
            status_at: Utc::now(),
            task_json: serde_json::json!({
                "id": "task-a",
                "contextId": "ctx-a",
                "status": {"state": "TASK_STATE_WORKING"}
            }),
        })
        .await
        .unwrap();
    Tenants {
        ws_a,
        channel_a,
        thread_a,
        target: target.id,
        alice,
        bob,
    }
}

/// What `reader` reads of workspace A through each reader-filtered listing,
/// as counts: threads paged, threads found, pending gates (list, page,
/// count), A2A tasks (list, count), open threads in the channel and the
/// workspace queue, open threads by occupancy, tombstones, posts in the
/// census, and pins on the target's backlinks.
async fn seen_by(store: &dyn Store, t: &Tenants, reader: MemberId) -> Vec<i64> {
    let len = |n: usize| n as i64;
    vec![
        len(store
            .page_threads_for_workspace(t.ws_a, None, 50, reader)
            .await
            .unwrap()
            .len()),
        len(store
            .find_threads_for_workspace(
                t.ws_a,
                WorkspaceThreadSearch {
                    limit: 50,
                    ..WorkspaceThreadSearch::for_reader(reader)
                },
            )
            .await
            .unwrap()
            .len()),
        len(store
            .list_pending_approval_gates(t.ws_a, reader, 50)
            .await
            .unwrap()
            .len()),
        len(store
            .page_pending_approval_gates(
                t.ws_a,
                PendingGateQuery {
                    limit: 50,
                    ..PendingGateQuery::for_reader(reader)
                },
            )
            .await
            .unwrap()
            .len()),
        store
            .count_pending_approval_gates(t.ws_a, PendingGateQuery::for_reader(reader))
            .await
            .unwrap(),
        len(store
            .list_a2a_tasks(
                t.ws_a,
                A2aTaskQuery {
                    limit: 50,
                    ..A2aTaskQuery::for_reader(reader)
                },
            )
            .await
            .unwrap()
            .len()),
        store
            .count_a2a_tasks(t.ws_a, A2aTaskQuery::for_reader(reader))
            .await
            .unwrap(),
        store
            .channel_queue_depth(t.channel_a, reader)
            .await
            .unwrap()
            .open,
        store
            .workspace_queue_depth(t.ws_a, reader, None, None, None)
            .await
            .unwrap()
            .open,
        store
            .channel_occupancy(t.channel_a, reader)
            .await
            .unwrap()
            .open,
        store
            .workspace_occupancy(t.ws_a, reader)
            .await
            .unwrap()
            .open,
        len(store
            .list_tombstones(t.ws_a, None, None, false, 50, reader)
            .await
            .unwrap()
            .len()),
        posted(
            &store
                .event_kind_census(t.ws_a, None, Some(t.thread_a), &[], reader)
                .await
                .unwrap(),
        ),
        len(store
            .list_message_backlinks(t.target, reader)
            .await
            .unwrap()
            .pins
            .len()),
    ]
}

async fn run_suite(store: &dyn Store) -> Tenants {
    let t = tenants(store).await;
    assert_eq!(
        seen_by(store, &t, t.alice).await,
        vec![1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 1],
        "A's member reads A's public thread through every listing"
    );
    assert_eq!(
        seen_by(store, &t, t.bob).await,
        vec![0; 14],
        "B's member reads nothing on A's threads through any listing"
    );
    t
}

/// Workspace A's threads that pass the shared listing filter for a NULL
/// reader bind, counted in SQL.
const NULL_READER_COUNT: &str = "SELECT COUNT(*) FROM maidan_threads t
     JOIN maidan_channels c ON c.id = t.channel_id
     WHERE c.workspace_id = {ws} AND {readable}";

fn null_reader_sql(ws: &str, member: &str, dm: &str) -> String {
    let readable = maidan_store::thread_access::readable_row("t.id", ws, member, dm);
    NULL_READER_COUNT
        .replace("{ws}", ws)
        .replace("{readable}", &readable)
}

#[tokio::test]
async fn reader_filtered_listings_hold_two_tenants_apart_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let t = run_suite(store.as_ref()).await;
    let hidden: i64 = sqlx::query_scalar(&null_reader_sql("?1", "?2", "?3"))
        .bind(t.ws_a.0)
        .bind(None::<uuid::Uuid>)
        .bind(DM_CHANNEL_NAME)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(hidden, 0, "a NULL reader sees no thread");
}

#[tokio::test]
async fn reader_filtered_listings_hold_two_tenants_apart_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let t = run_suite(store.as_ref()).await;
    let hidden: i64 = sqlx::query_scalar(&null_reader_sql("$1", "$2::uuid", "$3"))
        .bind(t.ws_a.0)
        .bind(None::<uuid::Uuid>)
        .bind(DM_CHANNEL_NAME)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(hidden, 0, "a NULL reader sees no thread");
}
