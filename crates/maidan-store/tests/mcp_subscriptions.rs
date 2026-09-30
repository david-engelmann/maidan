//! Stateless MCP resource subscriptions: the shared set every replica reads,
//! the same on both backends. Scoped by workspace, bounded per subscriber, and
//! lapsing unless a listener keeps extending them.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations, McpSubscriptionWatch, NewMcpSubscription};
use maidan_types::{MemberId, MemberKind, NewMember, NewWorkspace, WorkspaceId};
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
    SqliteStore::for_tests(pool)
}

async fn tenant(store: &dyn Store, name: &str) -> (WorkspaceId, MemberId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-agent"),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    (ws.id, member.id)
}

fn sub(
    subscriber: &str,
    tenant: Option<(WorkspaceId, MemberId)>,
    uri: &str,
    ttl: Duration,
) -> NewMcpSubscription {
    NewMcpSubscription {
        subscriber: subscriber.into(),
        workspace_id: tenant.map(|(ws, _)| ws),
        member_id: tenant.map(|(_, m)| m),
        uri: uri.into(),
        expires_at: Utc::now() + ttl,
    }
}

fn watch(subscriber: &str, uri: &str) -> McpSubscriptionWatch {
    McpSubscriptionWatch {
        subscriber: subscriber.into(),
        uri: uri.into(),
    }
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

async fn sorted_watchers(
    store: &dyn Store,
    ws: WorkspaceId,
    uris: &[&str],
    subscribers: &[&str],
) -> Vec<McpSubscriptionWatch> {
    let mut got = store
        .mcp_resource_watchers(ws, &strings(uris), &strings(subscribers), Utc::now())
        .await
        .expect("watchers");
    got.sort_by(|a, b| (&a.subscriber, &a.uri).cmp(&(&b.subscriber, &b.uri)));
    got
}

const THREAD: &str = "maidan://threads/t1";
const OTHER: &str = "maidan://threads/t2";
const ARTIFACT: &str = "maidan://artifacts/shared-sha";

async fn run_suite(store: &dyn Store) {
    let alpha = tenant(store, "alpha").await;
    let bravo = tenant(store, "bravo").await;
    let hour = Duration::hours(1);

    // Subscribing is idempotent; unsubscribing reports whether it was held.
    assert!(store
        .subscribe_mcp_resource(&sub("alice", Some(alpha), THREAD, hour), 8)
        .await
        .expect("subscribe"));
    assert!(store
        .subscribe_mcp_resource(&sub("alice", Some(alpha), THREAD, hour), 8)
        .await
        .expect("again"));
    assert_eq!(
        sorted_watchers(store, alpha.0, &[THREAD, OTHER], &["alice"]).await,
        vec![watch("alice", THREAD)]
    );
    assert!(!store
        .unsubscribe_mcp_resource("bob", THREAD)
        .await
        .expect("not bob's"));
    assert!(store
        .unsubscribe_mcp_resource("alice", THREAD)
        .await
        .expect("unsubscribe"));
    assert!(!store
        .unsubscribe_mcp_resource("alice", THREAD)
        .await
        .expect("gone"));
    assert!(sorted_watchers(store, alpha.0, &[THREAD], &["alice"])
        .await
        .is_empty());

    // Two tenants watching the same content-addressed URI: an update in one
    // workspace finds only that workspace's subscriber, even asked about both.
    // An auth-disabled caller's subscription belongs to no workspace.
    for (who, t) in [
        ("alice", Some(alpha)),
        ("bravo", Some(bravo)),
        ("bypass", None),
    ] {
        assert!(store
            .subscribe_mcp_resource(&sub(who, t, ARTIFACT, hour), 8)
            .await
            .expect("subscribe"));
    }
    let everyone = ["alice", "bravo", "bypass"];
    assert_eq!(
        sorted_watchers(store, alpha.0, &[ARTIFACT], &everyone).await,
        vec![watch("alice", ARTIFACT), watch("bypass", ARTIFACT)]
    );
    assert_eq!(
        sorted_watchers(store, bravo.0, &[ARTIFACT], &everyone).await,
        vec![watch("bravo", ARTIFACT), watch("bypass", ARTIFACT)]
    );
    // Only the subscribers asked about: those with a listener on this replica.
    assert_eq!(
        sorted_watchers(store, alpha.0, &[ARTIFACT], &["bravo"]).await,
        Vec::new()
    );
    assert!(store
        .mcp_resource_watchers(alpha.0, &[], &strings(&everyone), Utc::now())
        .await
        .expect("no uris")
        .is_empty());

    // The limit binds a new resource, not one already watched.
    for uri in ["maidan://threads/l1", "maidan://threads/l2"] {
        assert!(store
            .subscribe_mcp_resource(&sub("limited", Some(alpha), uri, hour), 2)
            .await
            .expect("under the limit"));
    }
    assert!(!store
        .subscribe_mcp_resource(&sub("limited", Some(alpha), "maidan://threads/l3", hour), 2)
        .await
        .expect("at the limit"));
    assert!(store
        .subscribe_mcp_resource(&sub("limited", Some(alpha), "maidan://threads/l1", hour), 2)
        .await
        .expect("already watched"));
    assert!(
        sorted_watchers(store, alpha.0, &["maidan://threads/l3"], &["limited"])
            .await
            .is_empty()
    );

    // A lapsed subscription is not delivered, cannot be revived, and is reaped;
    // it no longer counts against the limit.
    let past = Duration::seconds(-1);
    assert!(store
        .subscribe_mcp_resource(&sub("idle", Some(alpha), THREAD, past), 8)
        .await
        .expect("subscribe"));
    assert!(sorted_watchers(store, alpha.0, &[THREAD], &["idle"])
        .await
        .is_empty());
    assert_eq!(
        store
            .extend_mcp_resource_subscriptions(&strings(&["idle"]), Utc::now(), Utc::now() + hour)
            .await
            .expect("extend"),
        0
    );
    assert!(
        store
            .reap_mcp_resource_subscriptions(Utc::now())
            .await
            .expect("reap")
            >= 1
    );
    assert!(!store
        .unsubscribe_mcp_resource("idle", THREAD)
        .await
        .expect("reaped"));

    // A listener's extension keeps a subscription past its first expiry, and
    // a new subscribe moves the subscriber's others along with it.
    let soon = Duration::seconds(2);
    store
        .subscribe_mcp_resource(&sub("kept", Some(bravo), THREAD, soon), 8)
        .await
        .expect("subscribe");
    store
        .subscribe_mcp_resource(&sub("kept", Some(bravo), OTHER, soon), 8)
        .await
        .expect("subscribe");
    store
        .subscribe_mcp_resource(&sub("dropped", Some(bravo), THREAD, soon), 8)
        .await
        .expect("subscribe");
    assert_eq!(
        store
            .extend_mcp_resource_subscriptions(
                &strings(&["kept", "nobody"]),
                Utc::now(),
                Utc::now() + hour,
            )
            .await
            .expect("extend"),
        2
    );
    store
        .reap_mcp_resource_subscriptions(Utc::now() + Duration::seconds(10))
        .await
        .expect("reap");
    let later = Utc::now() + Duration::seconds(10);
    let mut left = store
        .mcp_resource_watchers(
            bravo.0,
            &strings(&[THREAD, OTHER]),
            &strings(&["kept", "dropped"]),
            later,
        )
        .await
        .expect("watchers");
    left.sort_by(|a, b| a.uri.cmp(&b.uri));
    assert_eq!(left, vec![watch("kept", THREAD), watch("kept", OTHER)]);

    store
        .subscribe_mcp_resource(&sub("moved", Some(bravo), THREAD, soon), 8)
        .await
        .expect("subscribe");
    store
        .subscribe_mcp_resource(&sub("moved", Some(bravo), OTHER, hour), 8)
        .await
        .expect("subscribe");
    let mut moved = store
        .mcp_resource_watchers(
            bravo.0,
            &strings(&[THREAD, OTHER]),
            &strings(&["moved"]),
            later,
        )
        .await
        .expect("watchers");
    moved.sort_by(|a, b| a.uri.cmp(&b.uri));
    assert_eq!(moved, vec![watch("moved", THREAD), watch("moved", OTHER)]);
}

#[tokio::test]
async fn mcp_subscriptions_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn mcp_subscriptions_postgres() {
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
}
