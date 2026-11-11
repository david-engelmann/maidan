//! Artifact visibility, on both backends: a blob is readable by its uploader
//! without a link, hidden from a same-workspace member until they can see a
//! live thread that links it, and hidden again when that access is revoked.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ArtifactKind, ChannelMemberRole, MemberKind, NewArtifact, NewChannel, NewMember, NewMessage,
    NewThread, NewWorkspace,
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

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "ws".into() })
        .await
        .expect("workspace");
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("alice");
    let bob = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("bob");
    let carol = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "carol".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("carol");
    let sha = "ab".repeat(32);
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.clone(),
                size_bytes: 4,
                mime_type: Some("text/plain".into()),
                filename: Some("notes.txt".into()),
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(alice.id),
            },
            Some(ws.id),
        )
        .await
        .expect("upload");

    assert!(
        store
            .artifact_visible_to(ws.id, &sha, alice.id)
            .await
            .expect("uploader"),
        "the uploader can read their blob without a link"
    );
    assert!(
        !store
            .artifact_visible_to(ws.id, &sha, bob.id)
            .await
            .expect("no link"),
        "a same-workspace member cannot read a blob they did not upload and cannot see a link to"
    );

    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "secret".into(),
            topic: None,
            private: true,
        })
        .await
        .expect("channel");
    store
        .add_channel_member(channel.id, alice.id, ChannelMemberRole::Member)
        .await
        .expect("alice joins");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("docs".into()),
            description: None,
            created_by: Some(alice.id),
        })
        .await
        .expect("thread");
    store
        .link_thread_artifact(thread.id, &sha, alice.id)
        .await
        .expect("link");
    assert!(
        !store
            .artifact_visible_to(ws.id, &sha, bob.id)
            .await
            .expect("private link"),
        "a link on a private channel bob is not in does not reveal the blob"
    );

    store
        .add_channel_member(channel.id, bob.id, ChannelMemberRole::Member)
        .await
        .expect("bob joins");
    assert!(
        store
            .artifact_visible_to(ws.id, &sha, bob.id)
            .await
            .expect("after join"),
        "membership reveals the linked blob"
    );

    store
        .remove_channel_member(channel.id, bob.id)
        .await
        .expect("revoke");
    assert!(
        !store
            .artifact_visible_to(ws.id, &sha, bob.id)
            .await
            .expect("after revoke"),
        "access-revoked: the linked artifact is hidden again"
    );

    // A sha named only in message metadata is the same rule, on both backends.
    let notes = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "notes".into(),
            topic: None,
            private: true,
        })
        .await
        .expect("notes");
    store
        .add_channel_member(notes.id, alice.id, ChannelMemberRole::Member)
        .await
        .expect("alice notes");
    let notes_thread = store
        .create_thread(NewThread {
            channel_id: notes.id,
            parent_thread_id: None,
            title: Some("pin".into()),
            description: None,
            created_by: Some(alice.id),
        })
        .await
        .expect("notes thread");
    let sha_meta = "cd".repeat(32);
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha_meta.clone(),
                size_bytes: 4,
                mime_type: Some("text/plain".into()),
                filename: Some("meta.txt".into()),
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(alice.id),
            },
            Some(ws.id),
        )
        .await
        .expect("upload meta");
    store
        .post_message(NewMessage {
            thread_id: notes_thread.id,
            author_id: alice.id,
            body: "pinned".into(),
            metadata: serde_json::json!({ "artifact_sha256": sha_meta }),
            content: None,
        })
        .await
        .expect("pin by field");
    assert!(
        !store
            .artifact_visible_to(ws.id, &sha_meta, bob.id)
            .await
            .expect("meta hidden"),
        "a metadata link on a private channel bob is not in does not reveal the blob"
    );
    store
        .add_channel_member(notes.id, bob.id, ChannelMemberRole::Member)
        .await
        .expect("bob notes");
    assert!(
        store
            .artifact_visible_to(ws.id, &sha_meta, bob.id)
            .await
            .expect("meta visible"),
        "membership reveals a metadata artifact_sha256 link"
    );

    let sha_arr = "ef".repeat(32);
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha_arr.clone(),
                size_bytes: 4,
                mime_type: Some("text/plain".into()),
                filename: Some("arr.txt".into()),
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(alice.id),
            },
            Some(ws.id),
        )
        .await
        .expect("upload arr");
    store
        .post_message(NewMessage {
            thread_id: notes_thread.id,
            author_id: alice.id,
            body: "also pinned".into(),
            metadata: serde_json::json!({ "artifacts": [{ "sha256": sha_arr }] }),
            content: None,
        })
        .await
        .expect("pin by array");
    assert!(
        store
            .artifact_visible_to(ws.id, &sha_arr, bob.id)
            .await
            .expect("arr visible"),
        "membership reveals a metadata artifacts[].sha256 link"
    );
    store
        .remove_channel_member(notes.id, bob.id)
        .await
        .expect("revoke notes");
    assert!(
        !store
            .artifact_visible_to(ws.id, &sha_meta, bob.id)
            .await
            .expect("meta revoked"),
        "access-revoked: a metadata link is hidden again"
    );
    assert!(
        !store
            .artifact_visible_to(ws.id, &sha_arr, bob.id)
            .await
            .expect("arr revoked"),
        "access-revoked: an artifacts[] link is hidden again"
    );

    // A DM thread link is visible to a participant and hidden from everyone else.
    let dm = store
        .open_dm_conversation(ws.id, alice.id, bob.id)
        .await
        .expect("dm");
    let sha_dm = "12".repeat(32);
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha_dm.clone(),
                size_bytes: 4,
                mime_type: Some("text/plain".into()),
                filename: Some("dm.txt".into()),
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(alice.id),
            },
            Some(ws.id),
        )
        .await
        .expect("upload dm");
    store
        .link_thread_artifact(dm.thread_id, &sha_dm, alice.id)
        .await
        .expect("dm link");
    assert!(
        store
            .artifact_visible_to(ws.id, &sha_dm, bob.id)
            .await
            .expect("dm participant"),
        "the other DM participant can read a blob linked on the conversation"
    );
    assert!(
        !store
            .artifact_visible_to(ws.id, &sha_dm, carol.id)
            .await
            .expect("dm outsider"),
        "a workspace member who is not in the DM cannot read a blob linked there"
    );
}

#[tokio::test]
async fn artifact_visible_to_follows_uploader_or_a_readable_link_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn artifact_visible_to_follows_uploader_or_a_readable_link_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
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
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::for_tests(pool)).await;
}
