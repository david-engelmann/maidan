//! One visibility rule for every read that lists a member's own rows or
//! serves an artifact: the mention inbox and its unread count, the mention
//! list, notifications and their unread count, the manager digest, the
//! pending-gate list and artifact reads. Each is checked on both backends
//! against a member of the same workspace who is not in the private channel
//! or the DM, a member of another workspace, and a member who leaves the
//! private channel after the row was written.

use std::collections::{BTreeSet, HashSet};

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalRisk, ArtifactKind, ChannelId, ChannelMemberRole, EventKind, LandColor, LandGateStatus,
    MemberId, MemberKind, NewApprovalGate, NewArtifact, NewChannel, NewMember, NewMessage,
    NewNotification, NewThread, NewWorkspace, ThreadId, WorkspaceId,
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

async fn member(store: &dyn Store, workspace_id: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member")
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
        .expect("channel")
        .id
}

async fn thread(store: &dyn Store, channel_id: ChannelId) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id
}

async fn post(
    store: &dyn Store,
    thread_id: ThreadId,
    author_id: MemberId,
    body: &str,
    metadata: serde_json::Value,
) -> maidan_types::MessageId {
    store
        .post_message(NewMessage {
            thread_id,
            author_id,
            body: body.into(),
            metadata,
            content: None,
        })
        .await
        .expect("post")
        .id
}

/// An artifact `workspace_id` holds, uploaded by `uploader`.
async fn artifact(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    uploader: MemberId,
    seed: &str,
) -> String {
    let sha = format!("{:0>64}", seed);
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.clone(),
                size_bytes: 3,
                mime_type: Some("text/plain".into()),
                filename: Some(format!("{seed}.txt")),
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(uploader),
            },
            Some(workspace_id),
        )
        .await
        .expect("artifact");
    sha
}

fn note(
    workspace_id: WorkspaceId,
    member_id: MemberId,
    source_log_id: i64,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
) -> NewNotification {
    NewNotification {
        workspace_id,
        member_id,
        kind: EventKind::ThreadResultSet,
        source_log_id,
        channel_id,
        thread_id,
        message_id: None,
        actor_id: None,
    }
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .expect("workspace")
        .id;
    let other_ws = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .expect("workspace")
        .id;
    // alice is in the private channel and the DM; carol is in neither; dave
    // is in the DM only; eve belongs to another workspace.
    let alice = member(store, ws, "alice").await;
    let carol = member(store, ws, "carol").await;
    let dave = member(store, ws, "dave").await;
    let eve = member(store, other_ws, "eve").await;

    let public = channel(store, ws, "general", false).await;
    let private = channel(store, ws, "board", true).await;
    store
        .add_channel_member(private, alice, ChannelMemberRole::Member)
        .await
        .expect("join");
    let open_thread = thread(store, public).await;
    let private_thread = thread(store, private).await;
    let dm = store
        .open_dm_conversation(ws, alice, dave)
        .await
        .expect("dm");

    // --- mentions and the inbox -------------------------------------------
    let secret = post(
        store,
        private_thread,
        alice,
        "@carol the acquisition price is 42",
        serde_json::json!({}),
    )
    .await;
    let whisper = post(
        store,
        dm.thread_id,
        alice,
        "@carol psst",
        serde_json::json!({}),
    )
    .await;
    let hello = post(
        store,
        open_thread,
        alice,
        "@carol hello",
        serde_json::json!({}),
    )
    .await;
    for message in [secret, whisper, hello] {
        store.record_mention(message, carol).await.expect("mention");
    }
    let inbox = store.list_member_inbox(carol, 50).await.expect("inbox");
    let bodies: Vec<_> = inbox
        .items
        .iter()
        .map(|i| i.message_body.as_str())
        .collect();
    assert_eq!(
        bodies,
        ["@carol hello"],
        "the inbox shows only what carol can read"
    );
    assert_eq!(inbox.unread_count, 1, "and counts only that");
    let mentions = store
        .list_mentions_for_member(carol, 50)
        .await
        .expect("mentions");
    assert_eq!(
        mentions.iter().map(|m| m.message_id).collect::<Vec<_>>(),
        [hello],
        "the mention list follows the same rule"
    );

    // --- notifications ------------------------------------------------------
    let mut log = 0;
    let mut next = || {
        log += 1;
        log
    };
    for row in [
        note(ws, carol, next(), Some(private), Some(private_thread)),
        note(ws, carol, next(), Some(private), None),
        note(
            ws,
            carol,
            next(),
            Some(dm_channel(store, dm.thread_id).await),
            Some(dm.thread_id),
        ),
        note(ws, carol, next(), Some(public), Some(open_thread)),
        note(ws, carol, next(), Some(public), None),
        note(ws, carol, next(), None, None),
        // A row naming another workspace's thread is never eve's to read.
        note(other_ws, eve, next(), Some(public), Some(open_thread)),
    ] {
        store.create_notification(row).await.expect("notification");
    }
    let seen: HashSet<_> = store
        .list_notifications(carol, false, 50)
        .await
        .expect("notifications")
        .into_iter()
        .map(|n| (n.channel_id, n.thread_id))
        .collect();
    let expected: HashSet<_> = [
        (Some(public), Some(open_thread)),
        (Some(public), None),
        (None, None),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        seen, expected,
        "carol's notifications are what she can read"
    );
    assert_eq!(
        store.unread_notification_count(carol).await.expect("count"),
        3,
        "the badge counts the same rows"
    );
    let digest = store
        .manager_digest_for_member(carol, chrono::Utc::now() - chrono::Duration::days(1))
        .await
        .expect("digest");
    assert!(
        digest
            .channels
            .iter()
            .all(|c| c.channel_id != Some(private)),
        "the manager digest names no private channel carol is not in"
    );
    assert!(
        store
            .list_notifications(eve, false, 50)
            .await
            .expect("notifications")
            .is_empty(),
        "another workspace's member reads none of this workspace's rows"
    );

    // Leaving the channel takes what was written while carol was in it.
    store
        .add_channel_member(private, carol, ChannelMemberRole::Member)
        .await
        .expect("join");
    assert_eq!(
        store
            .list_member_inbox(carol, 50)
            .await
            .expect("inbox")
            .items
            .len(),
        2
    );
    assert_eq!(
        store
            .list_notifications(carol, false, 50)
            .await
            .expect("n")
            .len(),
        5
    );
    store
        .remove_channel_member(private, carol)
        .await
        .expect("leave");
    assert_eq!(
        store
            .list_member_inbox(carol, 50)
            .await
            .expect("inbox")
            .items
            .len(),
        1
    );
    assert_eq!(
        store
            .list_notifications(carol, false, 50)
            .await
            .expect("n")
            .len(),
        3
    );

    // --- pending approval gates --------------------------------------------
    let gate = |thread_id: Option<ThreadId>, prompt: &'static str| NewApprovalGate {
        workspace_id: ws,
        thread_id,
        requested_by: alice,
        prompt: prompt.into(),
        schema: None,
        risk: ApprovalRisk::High,
    };
    for new in [
        gate(Some(private_thread), "ship the board deck"),
        gate(Some(dm.thread_id), "send the offer"),
        gate(Some(open_thread), "merge the fix"),
        gate(None, "rotate the key"),
    ] {
        store.create_approval_gate(&new).await.expect("gate");
    }
    let prompts = |gates: Vec<maidan_types::ApprovalGate>| {
        gates.into_iter().map(|g| g.prompt).collect::<BTreeSet<_>>()
    };
    assert_eq!(
        prompts(
            store
                .list_pending_approval_gates(ws, carol, 50)
                .await
                .expect("gates")
        ),
        ["merge the fix", "rotate the key"].map(String::from).into(),
        "carol sees no gate on a thread she cannot read"
    );
    assert_eq!(
        store
            .list_pending_approval_gates(ws, alice, 50)
            .await
            .expect("gates")
            .len(),
        4,
        "alice reads every thread"
    );
    assert!(
        store
            .list_pending_approval_gates(other_ws, eve, 50)
            .await
            .expect("gates")
            .is_empty(),
        "another workspace lists none of them"
    );

    // --- artifacts ----------------------------------------------------------
    let linked_private = artifact(store, ws, alice, "a1").await;
    store
        .link_thread_artifact(private_thread, &linked_private, alice)
        .await
        .expect("link");
    let linked_public = artifact(store, ws, alice, "a2").await;
    store
        .link_thread_artifact(open_thread, &linked_public, alice)
        .await
        .expect("link");
    let unlinked = artifact(store, ws, alice, "a3").await;
    let in_dm = artifact(store, ws, alice, "a4").await;
    post(
        store,
        dm.thread_id,
        alice,
        "the offer",
        serde_json::json!({ "artifacts": [{ "sha256": in_dm }] }),
    )
    .await;
    // Linked somewhere public too: the private link still hides it, so
    // linking a sha in public never unlocks a private channel's file.
    let both = artifact(store, ws, alice, "a5").await;
    store
        .link_thread_artifact(private_thread, &both, alice)
        .await
        .expect("link");
    store
        .link_thread_artifact(open_thread, &both, carol)
        .await
        .expect("link");
    let land_report = artifact(store, ws, alice, "a6").await;
    store
        .add_member_skill(alice, maidan_types::LAND_GATE_SKILL)
        .await
        .expect("skill");
    store
        .set_land_gate_pointer(
            private_thread,
            alice,
            LandGateStatus::Pass,
            Some(&land_report),
            Some(LandColor::Green),
        )
        .await
        .expect("land gate");
    let in_private_message = artifact(store, ws, dave, "a7").await;
    post(
        store,
        private_thread,
        alice,
        "see attached",
        serde_json::json!({ "artifact_sha256": in_private_message }),
    )
    .await;

    let visible = |ws: WorkspaceId, member: MemberId, sha: String| async move {
        store
            .artifact_visible_to(ws, member, &sha)
            .await
            .expect("visible")
    };
    for (sha, why) in [
        (&linked_private, "linked in a private channel"),
        (&in_dm, "named in a DM"),
        (&both, "linked in a private channel and a public one"),
        (&land_report, "a private thread's land-gate report"),
        (&in_private_message, "named by a private channel's message"),
    ] {
        assert!(!visible(ws, carol, sha.clone()).await, "carol: {why}");
    }
    assert!(
        visible(ws, carol, linked_public.clone()).await,
        "a public link"
    );
    assert!(
        visible(ws, carol, unlinked.clone()).await,
        "referenced nowhere"
    );
    for sha in [
        &linked_private,
        &in_dm,
        &both,
        &land_report,
        &in_private_message,
    ] {
        assert!(
            visible(ws, alice, sha.clone()).await,
            "alice reads them all"
        );
    }
    assert!(visible(ws, dave, in_dm.clone()).await, "dave is in the DM");
    assert!(
        visible(ws, dave, in_private_message.clone()).await,
        "dave uploaded it, wherever it went after"
    );
    assert!(
        !visible(ws, dave, linked_private.clone()).await,
        "dave is not in the channel"
    );
    for sha in [&linked_public, &unlinked] {
        assert!(
            !visible(other_ws, eve, sha.clone()).await,
            "another workspace holds no ref"
        );
        assert!(
            !visible(ws, eve, sha.clone()).await,
            "nor reads through this workspace's"
        );
    }
}

/// The thread backing a DM sits in the workspace's one DM channel.
async fn dm_channel(store: &dyn Store, thread_id: ThreadId) -> ChannelId {
    store
        .get_thread(thread_id)
        .await
        .expect("thread")
        .channel_id
}

#[tokio::test]
async fn every_member_scoped_read_and_artifact_read_follows_one_rule_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn every_member_scoped_read_and_artifact_read_follows_one_rule_postgres() {
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
