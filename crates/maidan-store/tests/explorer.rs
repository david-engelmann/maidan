//! Tombstone explorer, message backlinks, EventKind census.

use maidan_store::StoreError;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EventKind, MemberKind, NewChannel, NewMember, NewMessage, NewPin, NewReaction, NewReference,
    NewThread, NewVote, NewWorkspace, RefSide, RelationKind, TombstoneEntityKind,
};
use sqlx::sqlite::SqlitePoolOptions;

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

fn new_msg(
    thread_id: maidan_types::ThreadId,
    author: maidan_types::MemberId,
    body: &str,
) -> NewMessage {
    NewMessage {
        thread_id,
        author_id: author,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    }
}

async fn run_explorer_suite(store: &dyn Store) {
    let (ws, _) = store
        .create_workspace_with_event(NewWorkspace { name: "ex".into() })
        .await
        .expect("ws");
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("alice");
    let (channel, _) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let (thread, _) = store
        .create_thread_with_event(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .expect("th");
    let (live, _) = store
        .post_message_with_event(new_msg(thread.id, alice.id, "keep"), None)
        .await
        .expect("live");
    let (dead, _) = store
        .post_message_with_event(new_msg(thread.id, alice.id, "drop"), None)
        .await
        .expect("dead");
    store
        .tombstone_message_with_event(dead.id, None)
        .await
        .expect("tombstone");

    // Other workspace must not leak.
    let (other, _) = store
        .create_workspace_with_event(NewWorkspace {
            name: "other".into(),
        })
        .await
        .expect("ws2");
    let bob = store
        .create_member(NewMember {
            workspace_id: other.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("bob");
    let (och, _) = store
        .create_channel_with_event(NewChannel {
            workspace_id: other.id,
            name: "o".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("och");
    let (oth, _) = store
        .create_thread_with_event(NewThread {
            channel_id: och.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("oth");
    let (od, _) = store
        .post_message_with_event(new_msg(oth.id, bob.id, "elsewhere"), None)
        .await
        .expect("om");
    store
        .tombstone_message_with_event(od.id, None)
        .await
        .expect("other tombstone");

    let listed = store
        .list_tombstones(ws.id, None, None, false, 50)
        .await
        .expect("list");
    assert_eq!(listed.len(), 1, "only this workspace's tombstone");
    assert_eq!(listed[0].id, dead.id.0);
    assert_eq!(listed[0].entity_kind, TombstoneEntityKind::Message);
    assert!(listed[0].retained);
    assert_eq!(listed[0].author_id, Some(alice.id));
    assert_eq!(listed[0].thread_id, thread.id);
    assert_eq!(listed[0].channel_id, channel.id);
    assert!(listed.iter().all(|t| t.id != live.id.0));

    let by_thread = store
        .list_tombstones(ws.id, None, Some(thread.id), false, 50)
        .await
        .expect("thread filter");
    assert_eq!(by_thread.len(), 1);
    let empty_ch = store
        .list_tombstones(ws.id, Some(och.id), None, false, 50)
        .await
        .expect("wrong channel");
    assert!(empty_ch.is_empty());

    // Hard purge: default list drops it; include_purged reconstructs from the event.
    store.purge_message(dead.id).await.expect("purge");
    let after_purge = store
        .list_tombstones(ws.id, None, None, false, 50)
        .await
        .expect("after purge");
    assert!(after_purge.is_empty(), "purged row is no longer retained");
    let with_purged = store
        .list_tombstones(ws.id, None, None, true, 50)
        .await
        .expect("include_purged");
    assert_eq!(with_purged.len(), 1);
    assert_eq!(with_purged[0].id, dead.id.0);
    assert!(!with_purged[0].retained);
    assert!(with_purged[0].author_id.is_none());
    assert!(with_purged[0].source_log_id.is_some());

    // Backlinks: incoming RelationKind + pin/reaction/vote. Outgoing ref excluded.
    let (target, _) = store
        .post_message_with_event(new_msg(thread.id, alice.id, "target"), None)
        .await
        .expect("target");
    let (src, _) = store
        .post_message_with_event(new_msg(thread.id, alice.id, "src"), None)
        .await
        .expect("src");
    store
        .add_reference_with_event(NewReference {
            src_kind: RefSide::Message,
            src_id: src.id.0,
            dst_kind: RefSide::Message,
            dst_id: target.id.0,
            relation: RelationKind::Supports,
        })
        .await
        .expect("incoming ref");
    store
        .add_reference_with_event(NewReference {
            src_kind: RefSide::Message,
            src_id: target.id.0,
            dst_kind: RefSide::Message,
            dst_id: src.id.0,
            relation: RelationKind::Refutes,
        })
        .await
        .expect("outgoing ref");
    store
        .pin_message_with_event(NewPin {
            thread_id: thread.id,
            message_id: target.id,
            member_id: alice.id,
        })
        .await
        .expect("pin");
    store
        .add_reaction_with_event(NewReaction {
            message_id: target.id,
            member_id: alice.id,
            emoji: "👍".into(),
        })
        .await
        .expect("react");
    store
        .cast_vote_with_event(NewVote {
            message_id: target.id,
            member_id: alice.id,
            kind: "up".into(),
            confidence: None,
        })
        .await
        .expect("vote");

    let back = store
        .list_message_backlinks(target.id)
        .await
        .expect("backlinks");
    assert_eq!(back.message_id, target.id);
    assert_eq!(back.references.len(), 1, "only incoming RelationKind edges");
    assert_eq!(back.references[0].src_id, src.id.0);
    assert_eq!(back.references[0].relation, RelationKind::Supports);
    assert_eq!(back.pins.len(), 1);
    assert_eq!(back.reactions.len(), 1);
    assert_eq!(back.votes.len(), 1);

    let missing = store
        .list_message_backlinks(maidan_types::MessageId(uuid::Uuid::new_v4()))
        .await;
    assert!(matches!(missing, Err(StoreError::NotFound)));

    // Census: this workspace's events, scoped, deny-channels drops a channel.
    let census = store
        .event_kind_census(ws.id, None, None, &[])
        .await
        .expect("census");
    assert_eq!(census.workspace_id, ws.id);
    assert!(census.total > 0);
    let posted = census
        .counts
        .iter()
        .find(|c| c.kind == EventKind::MessagePosted)
        .expect("posted");
    assert!(posted.count >= 4, "live + dead + target + src");
    let tombstoned = census
        .counts
        .iter()
        .find(|c| c.kind == EventKind::MessageTombstoned)
        .expect("tombstoned");
    assert_eq!(tombstoned.count, 1);
    let other_census = store
        .event_kind_census(other.id, None, None, &[])
        .await
        .expect("other census");
    assert!(
        other_census
            .counts
            .iter()
            .find(|c| c.kind == EventKind::MessagePosted)
            .map(|c| c.count)
            .unwrap_or(0)
            >= 1
    );
    assert_ne!(census.total, other_census.total);

    let thread_census = store
        .event_kind_census(ws.id, None, Some(thread.id), &[])
        .await
        .expect("thread census");
    assert_eq!(thread_census.thread_id, Some(thread.id));
    assert!(thread_census.total > 0);
    assert!(thread_census.total < census.total);

    let denied = store
        .event_kind_census(ws.id, None, None, &[channel.id])
        .await
        .expect("deny");
    assert!(
        denied
            .counts
            .iter()
            .all(|c| c.kind != EventKind::MessagePosted),
        "deny_channels drops the only channel's message events"
    );
    assert!(
        denied
            .counts
            .iter()
            .any(|c| c.kind == EventKind::WorkspaceCreated),
        "workspace-level events survive deny_channels"
    );
}

/// The limit binds the **purged** half of the explorer too.
///
/// `list_purged` used to fetch and parse every `message_tombstoned` event in
/// the scope, leaving the caller to truncate — so a heavily-purged workspace
/// paid a full scan and a full parse for a `limit=10` read.
///
/// Its own workspace on purpose: the assertion is about counts, and sharing the
/// main suite's fixture made six extra purges change a census further down.
async fn run_purged_bound_suite(store: &dyn Store) {
    let (ws, _) = store
        .create_workspace_with_event(NewWorkspace {
            name: "purged-bound".into(),
        })
        .await
        .expect("ws");
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bound-alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("alice");
    let (channel, _) = store
        .create_channel_with_event(NewChannel {
            workspace_id: ws.id,
            name: "bound-c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .expect("thread");

    let mut purged = Vec::new();
    for i in 0..6 {
        let (m, _) = store
            .post_message_with_event(new_msg(thread.id, alice.id, &format!("doomed-{i}")), None)
            .await
            .expect("doomed");
        store
            .tombstone_message_with_event(m.id, None)
            .await
            .expect("tombstone");
        store.purge_message(m.id).await.expect("purge");
        purged.push(m.id.0);
    }

    let bounded = store
        .list_tombstones(ws.id, None, None, true, 3)
        .await
        .expect("bounded include_purged");
    assert_eq!(bounded.len(), 3, "the limit must bind the purged half");
    let newest: std::collections::HashSet<_> = purged.iter().rev().take(3).copied().collect();
    assert_eq!(
        bounded
            .iter()
            .map(|t| t.id)
            .collect::<std::collections::HashSet<_>>(),
        newest,
        "a bounded read must keep the newest, not an arbitrary prefix"
    );

    // An unbounded-enough read still sees all six, so the bound narrows the
    // result rather than losing rows.
    let all = store
        .list_tombstones(ws.id, None, None, true, 50)
        .await
        .expect("all");
    assert_eq!(all.len(), 6);
}

#[tokio::test]
async fn explorer_tombstones_backlinks_and_census_sqlite() {
    let store = sqlite().await;
    run_explorer_suite(&store).await;
    run_purged_bound_suite(&store).await;
}

#[tokio::test]
async fn explorer_tombstones_backlinks_and_census_postgres() {
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
    run_explorer_suite(&store).await;
    run_purged_bound_suite(&store).await;
}
