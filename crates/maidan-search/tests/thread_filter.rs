//! Search filters by thread read access in the query, on both backends: a
//! reader never gets another workspace's hits, nor a private channel's or a
//! DM's it is not in, and hidden rows never crowd a full page out, for
//! lexical and semantic search alike.

use std::{sync::Arc, time::Duration};

use maidan_search::{postgres::EMBEDDING_DIM, PostgresSearch, Search, SearchFilters};
use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;

const N: usize = 5;

fn vector() -> Vec<f32> {
    let mut v = vec![0.0; EMBEDDING_DIM];
    v[0] = 1.0;
    v
}

async fn member(store: &dyn Store, ws: WorkspaceId, handle: &str) -> Member {
    store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
}

async fn channel(store: &dyn Store, ws: WorkspaceId, name: &str, private: bool) -> Channel {
    store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .unwrap()
}

/// Posts `body` in a new thread of `channel` (or in `thread`), embeds it, and
/// returns the message.
async fn post(
    store: &dyn Store,
    search: &dyn Search,
    thread: ThreadId,
    author: MemberId,
    body: String,
) -> Message {
    let message = store
        .post_message(NewMessage {
            thread_id: thread,
            author_id: author,
            body,
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    search
        .upsert_embedding(message.id, "test-model", &vector())
        .await
        .unwrap();
    message
}

async fn thread_in(store: &dyn Store, channel: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: None,
            description: None,
        })
        .await
        .unwrap()
        .id
}

async fn run_suite(store: &dyn Store, search: &dyn Search) {
    // Two workspaces with the same text: A's reader sees only A.
    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let alice = member(store, ws_a.id, "alice").await;
    let carol = member(store, ws_b.id, "carol").await;
    let general_a = channel(store, ws_a.id, "general", false).await;
    let general_b = channel(store, ws_b.id, "general", false).await;
    let t = thread_in(store, general_a.id).await;
    post(store, search, t, alice.id, "alpha unique".into()).await;
    let t = thread_in(store, general_b.id).await;
    post(store, search, t, carol.id, "alpha unique".into()).await;
    let as_alice = SearchFilters {
        reader_id: Some(alice.id),
        ..Default::default()
    };
    let hits = search
        .search_messages(ws_a.id, "alpha", 10, &as_alice)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "lexical: only A's message");
    assert!(hits.iter().all(|h| h.workspace_id == ws_a.id));

    // In A: N visible messages, then N in a private channel and N in a DM the
    // reader is in neither of. A page of N is still N visible hits.
    let bob = member(store, ws_a.id, "bob").await;
    let dave = member(store, ws_a.id, "dave").await;
    let public = channel(store, ws_a.id, "public", false).await;
    let secret = channel(store, ws_a.id, "secret", true).await;
    store
        .add_channel_member(secret.id, bob.id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let dm = store
        .open_dm_conversation(ws_a.id, bob.id, dave.id)
        .await
        .unwrap();
    let mut visible = Vec::new();
    for i in 0..N {
        let t = thread_in(store, public.id).await;
        visible.push(
            post(store, search, t, alice.id, format!("beta visible {i}"))
                .await
                .id,
        );
    }
    for i in 0..N {
        let t = thread_in(store, secret.id).await;
        post(store, search, t, bob.id, format!("beta private {i}")).await;
        post(store, search, dm.thread_id, bob.id, format!("beta dm {i}")).await;
    }

    let lexical = search
        .search_messages(ws_a.id, "beta", N as i64, &as_alice)
        .await
        .unwrap();
    assert_eq!(lexical.len(), N, "lexical: a full page of visible hits");
    assert!(
        lexical.iter().all(|h| visible.contains(&h.message_id)),
        "lexical: no private or DM hit"
    );

    let semantic = search
        .semantic_search(ws_a.id, &vector(), (3 * N) as i64, &as_alice, "test-model")
        .await
        .unwrap();
    let semantic_beta: Vec<_> = semantic
        .iter()
        .filter(|h| h.body.contains("beta"))
        .collect();
    assert_eq!(
        semantic_beta.len(),
        N,
        "semantic: only the visible beta hits"
    );
    assert!(
        semantic_beta
            .iter()
            .all(|h| visible.contains(&h.message_id)),
        "semantic: no private or DM hit"
    );
    assert!(
        semantic.iter().all(|h| h.workspace_id == ws_a.id),
        "semantic: nothing from B"
    );
}

#[tokio::test]
async fn search_filters_by_thread_access_in_query_sqlite() {
    // The crate's pool options load sqlite-vec and turn on foreign keys on
    // every connection. A bare pool opened before `SqliteSearch` registers the
    // extension leaves its first connection without `vec_distance_cosine`.
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search = maidan_search::SqliteSearch::new(pool);
    run_suite(store.as_ref(), &search).await;
}

#[tokio::test]
async fn search_filters_by_thread_access_in_query_postgres() {
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
    let search = PostgresSearch::new(pool);
    run_suite(store.as_ref(), &search).await;
}
