//! Workspace thread paging and the explorer listings filter by thread read
//! access in the query, on both backends: a reader never gets another
//! workspace's rows, nor a private channel's or a DM's it is not in, and
//! hidden rows never crowd a full page out.

use std::{collections::HashSet, sync::Arc, time::Duration};

use maidan_store::{prelude::*, run_postgres_migrations, run_sqlite_migrations};
use maidan_types::*;

const N: i64 = 5;

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

fn posted_count(census: &KindCensus) -> i64 {
    census
        .counts
        .iter()
        .find(|c| c.kind == EventKind::MessagePosted)
        .map(|c| c.count)
        .unwrap_or(0)
}

async fn run_suite(store: &dyn Store) {
    // Workspace A: alice reads only public threads, bob is in the private
    // channel and the DM, dave is only in the DM.
    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let alice = member(store, ws_a.id, "alice").await;
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

    // Workspace B: carol reads its public channel only; eve holds its
    // private channel and a DM with carol.
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let carol = member(store, ws_b.id, "carol").await;
    let eve = member(store, ws_b.id, "eve").await;
    let general_b = channel(store, ws_b.id, "general", false).await;
    let secret_b = channel(store, ws_b.id, "secret-b", true).await;
    store
        .add_channel_member(secret_b.id, eve.id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let dm_b = store
        .open_dm_conversation(ws_b.id, carol.id, eve.id)
        .await
        .unwrap();

    // ---- page_threads_for_workspace -------------------------------------
    let mut public_threads = HashSet::new();
    for _ in 0..N {
        public_threads.insert(thread_in(store, public.id).await);
    }
    let mut secret_threads = HashSet::new();
    for _ in 0..N {
        secret_threads.insert(thread_in(store, secret.id).await);
    }
    let mut b_threads = HashSet::new();
    for _ in 0..2 {
        b_threads.insert(thread_in(store, general_b.id).await);
    }
    for _ in 0..2 {
        thread_in(store, secret_b.id).await;
    }

    // A full page of N for alice despite N private threads and a DM thread
    // sitting in the same workspace.
    let page = store
        .page_threads_for_workspace(ws_a.id, None, N, Some(alice.id))
        .await
        .unwrap();
    assert_eq!(page.len() as i64, N, "alice's page stays full-length");
    assert!(
        page.iter().all(|t| public_threads.contains(&t.id)),
        "alice sees only public threads"
    );

    // Bob reads everything in A: public, private and the DM thread.
    let page = store
        .page_threads_for_workspace(ws_a.id, None, 2 * N + 1, Some(bob.id))
        .await
        .unwrap();
    assert_eq!(page.len() as i64, 2 * N + 1, "bob sees all of A");
    assert!(page.iter().any(|t| t.id == dm.thread_id));
    assert!(secret_threads
        .iter()
        .all(|id| page.iter().any(|t| &t.id == id)));

    // No reader: the unrestricted view matches bob's here.
    let page = store
        .page_threads_for_workspace(ws_a.id, None, 2 * N + 1, None)
        .await
        .unwrap();
    assert_eq!(page.len() as i64, 2 * N + 1, "no reader sees all of A");

    // Two tenants: carol's page of B holds only B threads she may read, and
    // alice paging A never surfaces a B thread.
    let page_b = store
        .page_threads_for_workspace(ws_b.id, None, 50, Some(carol.id))
        .await
        .unwrap();
    assert_eq!(page_b.len(), 3, "carol sees B's public threads and her DM");
    assert!(page_b
        .iter()
        .all(|t| b_threads.contains(&t.id) || t.id == dm_b.thread_id));
    let page = store
        .page_threads_for_workspace(ws_a.id, None, 50, Some(alice.id))
        .await
        .unwrap();
    assert!(
        page.iter().all(|t| !b_threads.contains(&t.id)),
        "no B thread leaks into A's page"
    );

    // ---- posts for the census, tombstone and backlink sections ----------
    let target_thread = *public_threads.iter().next().unwrap();
    let target = post(store, target_thread, alice.id, "target").await;
    let src_public = post(store, target_thread, alice.id, "src public").await;
    let secret_thread = *secret_threads.iter().next().unwrap();
    let src_secret = post(store, secret_thread, bob.id, "src secret").await;
    let target_secret = post(store, secret_thread, bob.id, "target secret").await;
    let src_dm = post(store, dm.thread_id, bob.id, "src dm").await;
    // 2 public + 2 secret + 1 DM MessagePosted events in A so far.
    let b_msg = post(store, *b_threads.iter().next().unwrap(), carol.id, "b").await;
    let b_secret_thread = thread_in(store, secret_b.id).await;
    let b_secret_msg = post(store, b_secret_thread, eve.id, "b secret").await;

    // ---- event_kind_census ----------------------------------------------
    let census_none = store
        .event_kind_census(ws_a.id, None, None, &[], None)
        .await
        .unwrap();
    let census_alice = store
        .event_kind_census(ws_a.id, None, None, &[], Some(alice.id))
        .await
        .unwrap();
    let census_bob = store
        .event_kind_census(ws_a.id, None, None, &[], Some(bob.id))
        .await
        .unwrap();
    assert_eq!(posted_count(&census_none), 5, "all A posts counted");
    assert_eq!(
        posted_count(&census_alice),
        2,
        "alice counts public posts only"
    );
    assert_eq!(posted_count(&census_bob), 5, "bob counts every A post");
    // A reader from another tenant counts none of A's posts, not even its
    // public ones: the store's rule requires the reader to belong to the
    // thread's workspace, whatever workspace the query names.
    let census_carol = store
        .event_kind_census(ws_a.id, None, None, &[], Some(carol.id))
        .await
        .unwrap();
    assert_eq!(
        posted_count(&census_carol),
        0,
        "carol counts none of another tenant's posts"
    );
    let census_b = store
        .event_kind_census(ws_b.id, None, None, &[], Some(carol.id))
        .await
        .unwrap();
    assert_eq!(
        posted_count(&census_b),
        1,
        "carol counts only B's public post"
    );

    // ---- list_message_backlinks ------------------------------------------
    for (src, relation) in [
        (src_public.id, RelationKind::Supports),
        (src_secret.id, RelationKind::Refutes),
        (src_dm.id, RelationKind::Supports),
    ] {
        store
            .add_reference_with_event(NewReference {
                src_kind: RefSide::Message,
                src_id: src.0,
                dst_kind: RefSide::Message,
                dst_id: target.id.0,
                relation,
            })
            .await
            .unwrap();
    }
    store
        .add_reference_with_event(NewReference {
            src_kind: RefSide::Message,
            src_id: src_public.id.0,
            dst_kind: RefSide::Message,
            dst_id: target_secret.id.0,
            relation: RelationKind::Supports,
        })
        .await
        .unwrap();
    store
        .pin_message_with_event(NewPin {
            thread_id: target_thread,
            message_id: target.id,
            member_id: bob.id,
        })
        .await
        .unwrap();

    let back = store
        .list_message_backlinks(target.id, Some(alice.id))
        .await
        .unwrap();
    assert_eq!(
        back.references.len(),
        1,
        "alice sees only the public source"
    );
    assert_eq!(back.references[0].src_id, src_public.id.0);
    assert_eq!(back.pins.len(), 1, "pins ride on the readable target");

    let back = store
        .list_message_backlinks(target.id, Some(bob.id))
        .await
        .unwrap();
    assert_eq!(back.references.len(), 3, "bob sees every source");
    let back = store.list_message_backlinks(target.id, None).await.unwrap();
    assert_eq!(back.references.len(), 3, "no reader sees every source");

    // An unreadable target answers empty, not a leak.
    let back = store
        .list_message_backlinks(target_secret.id, Some(alice.id))
        .await
        .unwrap();
    assert!(
        back.references.is_empty(),
        "unreadable target: no references"
    );
    assert!(back.pins.is_empty(), "unreadable target: no pins");
    let back = store
        .list_message_backlinks(target_secret.id, Some(bob.id))
        .await
        .unwrap();
    assert_eq!(
        back.references.len(),
        1,
        "bob sees the public source on his target"
    );

    // ---- list_tombstones --------------------------------------------------
    for m in [&target, &src_secret, &src_dm, &b_msg, &b_secret_msg] {
        store
            .tombstone_message_with_event(m.id, None)
            .await
            .unwrap();
    }
    let tombs = store
        .list_tombstones(ws_a.id, None, None, false, 50, Some(alice.id))
        .await
        .unwrap();
    assert_eq!(tombs.len(), 1, "alice sees only the public tombstone");
    assert_eq!(tombs[0].id, target.id.0);
    let tombs = store
        .list_tombstones(ws_a.id, None, None, false, 50, Some(bob.id))
        .await
        .unwrap();
    assert_eq!(tombs.len(), 3, "bob sees public, private and DM tombstones");
    let tombs = store
        .list_tombstones(ws_b.id, None, None, false, 50, Some(carol.id))
        .await
        .unwrap();
    assert_eq!(tombs.len(), 1, "carol sees only B's public tombstone");
    assert_eq!(tombs[0].id, b_msg.id.0);

    // Purged rows keep the filter: the events remain, the messages are gone.
    store.purge_message(src_secret.id).await.unwrap();
    store.purge_message(src_dm.id).await.unwrap();
    let tombs = store
        .list_tombstones(ws_a.id, None, None, true, 50, Some(alice.id))
        .await
        .unwrap();
    assert_eq!(tombs.len(), 1, "purged private and DM rows stay hidden");
    let tombs = store
        .list_tombstones(ws_a.id, None, None, true, 50, Some(bob.id))
        .await
        .unwrap();
    assert_eq!(tombs.len(), 3, "bob still sees the purged rows");
    assert_eq!(
        tombs.iter().filter(|t| !t.retained).count(),
        2,
        "two of bob's rows are the purged reconstructions"
    );

    // ---- full-page tombstones in a fresh workspace -----------------------
    let ws_c = store
        .create_workspace(NewWorkspace { name: "c".into() })
        .await
        .unwrap();
    let ann = member(store, ws_c.id, "ann").await;
    let ben = member(store, ws_c.id, "ben").await;
    let pub_c = channel(store, ws_c.id, "pub-c", false).await;
    let sec_c = channel(store, ws_c.id, "sec-c", true).await;
    store
        .add_channel_member(sec_c.id, ben.id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let pub_thread_c = thread_in(store, pub_c.id).await;
    let sec_thread_c = thread_in(store, sec_c.id).await;
    let mut visible_tombs = HashSet::new();
    for i in 0..N {
        let m = post(store, pub_thread_c, ann.id, &format!("visible {i}")).await;
        store
            .tombstone_message_with_event(m.id, None)
            .await
            .unwrap();
        visible_tombs.insert(m.id.0);
        let m = post(store, sec_thread_c, ben.id, &format!("hidden {i}")).await;
        store
            .tombstone_message_with_event(m.id, None)
            .await
            .unwrap();
    }
    let tombs = store
        .list_tombstones(ws_c.id, None, None, false, N, Some(ann.id))
        .await
        .unwrap();
    assert_eq!(tombs.len() as i64, N, "a page of N stays full-length");
    assert!(
        tombs.iter().all(|t| visible_tombs.contains(&t.id)),
        "no hidden tombstone in the page"
    );
}

#[tokio::test]
async fn workspace_and_explorer_filter_by_thread_access_in_query_sqlite() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool));
    run_suite(store.as_ref()).await;
}

#[tokio::test]
async fn workspace_and_explorer_filter_by_thread_access_in_query_postgres() {
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
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool));
    run_suite(store.as_ref()).await;
}
