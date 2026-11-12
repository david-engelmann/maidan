//! Channel listing (`visible_channels`) and the search pre-filter
//! (`private_channel_deny_set`) read membership with one store query, not one
//! `channel_is_member` read per private channel (PERF-001, PERF-003). The rule
//! is checked on both backends with a second workspace; the round trips are
//! pinned on Postgres, whose driver logs each statement on the calling task.

use std::collections::{BTreeSet, HashSet};

use maidan_auth::{private_channel_deny_set, visible_channels, AuthContext};
use maidan_store::test_support::statements;
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, ChannelMemberRole, MemberId, MemberKind, NewChannel, NewMember, NewWorkspace,
    WorkspaceId, DM_CHANNEL_NAME,
};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
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

fn reader(member_id: MemberId, workspace_id: WorkspaceId) -> AuthContext {
    AuthContext::from_session(member_id, workspace_id, Vec::new())
}

async fn listed(store: &dyn Store, auth: &AuthContext, ws: WorkspaceId) -> BTreeSet<String> {
    visible_channels(store, auth, ws)
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect()
}

async fn run_suite(store: &dyn Store) {
    let a = store
        .create_workspace(NewWorkspace {
            name: "list-a".into(),
        })
        .await
        .unwrap()
        .id;
    let b = store
        .create_workspace(NewWorkspace {
            name: "list-b".into(),
        })
        .await
        .unwrap()
        .id;
    let alice = member(store, a, "alice").await;
    let bob = member(store, a, "bob").await;
    let carol = member(store, b, "carol").await;
    channel(store, a, "general", false).await;
    channel(store, a, DM_CHANNEL_NAME, true).await;
    let inner = channel(store, a, "inner", true).await;
    let outer = channel(store, a, "outer", true).await;
    let b_secret = channel(store, b, "b-secret", true).await;
    store
        .add_channel_member(inner, alice, ChannelMemberRole::Member)
        .await
        .unwrap();
    store
        .add_channel_member(b_secret, carol, ChannelMemberRole::Member)
        .await
        .unwrap();

    let names = |list: &[&str]| {
        list.iter()
            .map(|n| (*n).to_owned())
            .collect::<BTreeSet<_>>()
    };
    assert_eq!(
        listed(store, &reader(alice, a), a).await,
        names(&["general", DM_CHANNEL_NAME, "inner"]),
        "alice sees public, the DM channel and her private channel"
    );
    assert_eq!(
        listed(store, &reader(bob, a), a).await,
        names(&["general", DM_CHANNEL_NAME]),
    );
    assert_eq!(
        listed(store, &AuthContext::bypass(), a).await,
        names(&["general", DM_CHANNEL_NAME, "inner", "outer"]),
        "bypass sees every channel"
    );
    // carol's membership of a B channel opens nothing in A.
    assert_eq!(
        listed(store, &reader(carol, b), a).await,
        names(&["general", DM_CHANNEL_NAME]),
    );

    let deny = |v: Vec<ChannelId>| v.into_iter().collect::<HashSet<_>>();
    assert_eq!(
        deny(
            private_channel_deny_set(store, &reader(alice, a), a)
                .await
                .unwrap()
        ),
        HashSet::from([outer]),
    );
    assert_eq!(
        deny(
            private_channel_deny_set(store, &reader(bob, a), a)
                .await
                .unwrap()
        ),
        HashSet::from([inner, outer]),
        "the DM channel is never in the deny set"
    );
    assert_eq!(
        deny(
            private_channel_deny_set(store, &reader(carol, b), b)
                .await
                .unwrap()
        ),
        HashSet::new(),
    );
    assert!(private_channel_deny_set(store, &AuthContext::bypass(), a)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn channel_listing_follows_membership_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn channel_listing_follows_membership_in_one_round_trip_postgres() {
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;

    let ws = store
        .create_workspace(NewWorkspace {
            name: "list-count".into(),
        })
        .await
        .unwrap()
        .id;
    let dave = member(&store, ws, "dave").await;
    let auth = reader(dave, ws);
    for i in 0..12 {
        let id = channel(&store, ws, &format!("p{i}"), true).await;
        if i % 2 == 0 {
            store
                .add_channel_member(id, dave, ChannelMemberRole::Member)
                .await
                .unwrap();
        }
    }
    let (visible, listing) = statements::count(visible_channels(&store, &auth, ws)).await;
    assert_eq!(visible.unwrap().len(), 6);
    assert_eq!(listing, 1, "listing 12 private channels is one statement");
    let (deny, prefilter) = statements::count(private_channel_deny_set(&store, &auth, ws)).await;
    assert_eq!(deny.unwrap().len(), 6);
    assert_eq!(
        prefilter, 1,
        "the deny set over 12 private channels is one statement"
    );
}
