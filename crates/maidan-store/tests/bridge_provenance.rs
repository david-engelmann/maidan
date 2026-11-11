//! Bridged messages carry a server-set provenance, and bridge ingress is keyed
//! on (workspace, origin, external id) so a redelivery posts nothing
//! (RED-007, RED-002). Both backends.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EditMessage, MemberKind, MessageOrigin, MessageProvenance, NewChannel, NewMember, NewMessage,
    NewThread, NewWorkspace, ThreadId, WorkspaceId,
};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
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

/// A workspace with one member and one thread.
async fn seed(store: &dyn Store, name: &str) -> (WorkspaceId, maidan_types::MemberId, ThreadId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "linker".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("bridged".into()),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread");
    (ws.id, member.id, thread.id)
}

fn text(thread_id: ThreadId, author_id: maidan_types::MemberId, body: &str) -> NewMessage {
    NewMessage {
        thread_id,
        author_id,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    }
}

async fn run_suite(store: &dyn Store) {
    let (ws, member, thread) = seed(store, "acme").await;
    let comment = MessageProvenance::delivered(MessageOrigin::Github, "1001");

    // The first delivery posts, stamped with where it came from.
    let (posted, _) = store
        .post_bridged_message_with_event(text(thread, member, "octocat: hi"), comment.clone())
        .await
        .expect("post")
        .expect("the first delivery posts");
    assert_eq!(posted.provenance.as_ref(), Some(&comment));
    let read = store.get_message(posted.id).await.expect("get");
    assert_eq!(
        read.provenance.as_ref(),
        Some(&comment),
        "and it reads back"
    );

    // A redelivery of the same comment writes nothing.
    assert!(store
        .post_bridged_message_with_event(text(thread, member, "octocat: hi"), comment.clone())
        .await
        .expect("redeliver")
        .is_none());
    assert_eq!(
        store.list_messages(thread, 50).await.expect("list").len(),
        1
    );

    // The key is per origin: a Slack event with the same id is its own.
    let slack = MessageProvenance::delivered(MessageOrigin::Slack, "1001");
    assert!(store
        .post_bridged_message_with_event(text(thread, member, "U1: hi"), slack)
        .await
        .expect("slack")
        .is_some());

    // GitHub egress takes the key for a comment it posted; the webhook that
    // brings that comment back posts nothing.
    assert!(store
        .record_bridge_receipt(ws, MessageOrigin::Github, "2002")
        .await
        .expect("record"));
    assert!(
        !store
            .record_bridge_receipt(ws, MessageOrigin::Github, "2002")
            .await
            .expect("record again"),
        "a key is taken once"
    );
    assert!(store
        .post_bridged_message_with_event(
            text(thread, member, "maidan: echo"),
            MessageProvenance::delivered(MessageOrigin::Github, "2002"),
        )
        .await
        .expect("echo")
        .is_none());

    // A sentence the server writes about a bridge has no key: each one posts.
    for _ in 0..2 {
        let (notice, _) = store
            .post_bridged_message_with_event(
                text(thread, member, "o/r#7 closed without merging"),
                MessageProvenance::reported(MessageOrigin::Github),
            )
            .await
            .expect("notice")
            .expect("a reported sentence always posts");
        assert_eq!(
            notice.provenance.as_ref().map(|p| p.origin),
            Some(MessageOrigin::Github)
        );
        assert_eq!(
            notice
                .provenance
                .as_ref()
                .and_then(|p| p.external_id.clone()),
            None
        );
    }

    // A member's own post has none, whatever its metadata says.
    let (own, _) = store
        .post_message_to_live_thread_with_event(
            NewMessage {
                metadata: serde_json::json!({ "github": { "user": "octocat" }, "slack": {} }),
                ..text(thread, member, "forged")
            },
            None,
        )
        .await
        .expect("member post");
    assert!(own.provenance.is_none());

    // An edit keeps the provenance, even one that clears the metadata.
    let (edited, _) = store
        .edit_message_with_event(
            posted.id,
            member,
            EditMessage {
                body: "octocat: edited".into(),
                metadata: serde_json::json!({}),
                content: None,
            },
            None,
        )
        .await
        .expect("edit");
    assert_eq!(edited.provenance.as_ref(), Some(&comment));

    // A tombstone does not reopen the key.
    store.tombstone_message(posted.id).await.expect("tombstone");
    assert!(store
        .post_bridged_message_with_event(text(thread, member, "octocat: hi"), comment.clone())
        .await
        .expect("after tombstone")
        .is_none());

    // Two tenants: the same comment id in another workspace is that
    // workspace's own, and posts there.
    let (_, other_member, other_thread) = seed(store, "globex").await;
    let (theirs, _) = store
        .post_bridged_message_with_event(text(other_thread, other_member, "octocat: hi"), comment)
        .await
        .expect("other workspace")
        .expect("the same external id in another workspace posts");
    assert_eq!(theirs.thread_id, other_thread);
    assert_eq!(
        store
            .list_messages(other_thread, 50)
            .await
            .expect("list other")
            .len(),
        1
    );
}

#[tokio::test]
async fn bridged_messages_carry_provenance_and_post_once_per_key_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn bridged_messages_carry_provenance_and_post_once_per_key_postgres() {
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
            maidan_store::test_support::docker::skip_start_failure(err).await;
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
}
