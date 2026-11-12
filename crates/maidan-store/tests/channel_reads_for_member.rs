//! The batched reads behind channel listing and search naming:
//! `list_channels_with_membership` and `display_names`. Each is checked on
//! both backends with two workspaces, so an id from one workspace never
//! answers for the other, and on Postgres each is pinned to one statement.

use std::collections::BTreeMap;

use maidan_store::test_support::statements;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, ChannelMemberRole, MemberId, MemberKind, NewChannel, NewMember, NewThread,
    NewWorkspace, ThreadId, WorkspaceId,
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

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("workspace")
        .id
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

async fn thread(store: &dyn Store, channel_id: ChannelId, title: Option<&str>) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id: None,
            title: title.map(Into::into),
            description: None,
            created_by: None,
        })
        .await
        .expect("thread")
        .id
}

async fn membership(store: &dyn Store, ws: WorkspaceId, who: MemberId) -> BTreeMap<String, bool> {
    store
        .list_channels_with_membership(ws, who)
        .await
        .expect("list with membership")
        .into_iter()
        .map(|(channel, is_member)| (channel.name, is_member))
        .collect()
}

async fn run_suite(store: &dyn Store) {
    let a = workspace(store, "reads-a").await;
    let b = workspace(store, "reads-b").await;
    let alice = member(store, a, "alice").await;
    let bob = member(store, a, "bob").await;
    let carol = member(store, b, "carol").await;
    let general = channel(store, a, "general", false).await;
    let inner = channel(store, a, "inner", true).await;
    channel(store, a, "outer", true).await;
    let b_secret = channel(store, b, "b-secret", true).await;
    store
        .add_channel_member(inner, alice, ChannelMemberRole::Member)
        .await
        .expect("alice joins inner");
    store
        .add_channel_member(b_secret, carol, ChannelMemberRole::Member)
        .await
        .expect("carol joins b-secret");

    let want = |rows: &[(&str, bool)]| -> BTreeMap<String, bool> {
        rows.iter().map(|(n, m)| ((*n).to_owned(), *m)).collect()
    };
    assert_eq!(
        membership(store, a, alice).await,
        want(&[("general", false), ("inner", true), ("outer", false)]),
        "alice's membership flags in her workspace"
    );
    assert_eq!(
        membership(store, a, bob).await,
        want(&[("general", false), ("inner", false), ("outer", false)]),
        "bob is in no private channel"
    );
    assert_eq!(
        membership(store, a, carol).await,
        want(&[("general", false), ("inner", false), ("outer", false)]),
        "a member of workspace B is a member of nothing in A"
    );
    assert_eq!(
        membership(store, b, alice).await,
        want(&[("b-secret", false)]),
        "workspace B lists only its own channels"
    );

    let plan = thread(store, inner, Some("Plan")).await;
    let untitled = thread(store, general, None).await;
    let b_thread = thread(store, b_secret, Some("B plan")).await;
    let missing = ThreadId(uuid::Uuid::now_v7());
    let names = store
        .display_names(
            a,
            &[general, inner, b_secret],
            &[plan, untitled, b_thread, missing],
            &[alice, carol, MemberId(uuid::Uuid::now_v7())],
        )
        .await
        .expect("display names");
    assert_eq!(
        names.channels.len(),
        2,
        "only workspace A's channels: {names:?}"
    );
    assert_eq!(names.channels[&general], "general");
    assert_eq!(names.channels[&inner], "inner");
    assert_eq!(names.threads.len(), 1, "only A's titled thread: {names:?}");
    assert_eq!(names.threads[&plan], "Plan");
    assert_eq!(names.members.len(), 1, "only A's member: {names:?}");
    assert_eq!(names.members[&alice], "alice");

    let none = store.display_names(a, &[], &[], &[]).await.expect("no ids");
    assert_eq!(none, maidan_store::DisplayNames::default());
}

#[tokio::test]
async fn channel_reads_for_a_member_stay_in_their_workspace_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn channel_reads_for_a_member_stay_in_their_workspace_postgres() {
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
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;

    // One statement each, however many channels and ids there are.
    let ws = workspace(&store, "reads-count").await;
    let who = member(&store, ws, "dave").await;
    let mut channels = Vec::new();
    let mut threads = Vec::new();
    for i in 0..12 {
        let id = channel(&store, ws, &format!("p{i}"), true).await;
        threads.push(thread(&store, id, Some("t")).await);
        channels.push(id);
    }
    let (listed, listing) = statements::count(store.list_channels_with_membership(ws, who)).await;
    assert_eq!(listed.expect("list").len(), 12);
    assert_eq!(
        listing, 1,
        "listing 12 channels with membership is one statement"
    );
    let (named, naming) =
        statements::count(store.display_names(ws, &channels, &threads, &[who])).await;
    assert_eq!(named.expect("names").threads.len(), 12);
    assert_eq!(naming, 1, "naming 25 ids is one statement");
}
