//! A dead holder's claim comes back on time, and every claim is leased.
//!
//! The reaper (`claim_reaper::sweep_once`) frees a lapsed lease on an idle
//! channel, with nobody calling `claim_next`, and publishes the `ClaimExpired`
//! for the holder on the bus — on SQLite and on Postgres. Over REST a
//! `claim-next` that names no lease gets the server default, and a lease or
//! renewal outside 1 s..7 days is a 400 problem. A leased claim nobody
//! acknowledged gets one `ClaimUnacknowledged` on the bus, and the thread's
//! owner is notified of the stuck work. A hung claim that ran its thread past
//! `max_wall_secs` is stopped by the tick: `ClaimFailed` on the bus and the run
//! in the DLQ, though the agent never reported usage.

mod common;

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{claim_reaper, notification_router, router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    BudgetLimits, ChannelId, Event, EventFilter, EventKind, MemberId, MemberKind, NewApiToken,
    NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> (Arc<dyn Store>, Arc<dyn maidan_search::Search>) {
    let pool = SqlitePoolOptions::new()
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    (store, search)
}

struct Room {
    workspace: WorkspaceId,
    channel: ChannelId,
    holder: MemberId,
}

async fn room(store: &dyn Store) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let holder = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "holder".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        workspace: ws.id,
        channel: channel.id,
        holder: holder.id,
    }
}

async fn new_thread(store: &dyn Store, channel: ChannelId, title: &str) {
    store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
        })
        .await
        .unwrap();
}

/// The reaper frees a lapsed lease on a channel nobody is claiming from and
/// publishes `ClaimExpired` for the dead holder; a live lease is untouched,
/// and a second sweep finds nothing.
async fn reaper_frees_a_lapsed_lease_and_publishes_claim_expired(
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
) {
    let dir = tempfile::tempdir().unwrap();
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        bus.clone(),
        search,
    );
    let r = room(store.as_ref()).await;
    new_thread(store.as_ref(), r.channel, "dead").await;
    new_thread(store.as_ref(), r.channel, "alive").await;
    let dead = store
        .claim_next_thread(r.channel, r.holder, Some(3600))
        .await
        .unwrap()
        .unwrap();
    let alive = store
        .claim_next_thread(r.channel, r.holder, Some(3600))
        .await
        .unwrap()
        .unwrap();
    // The holder of `dead` stopped renewing.
    store
        .renew_claim(dead.id, r.holder, dead.claim_lease_id.unwrap(), -5)
        .await
        .unwrap();

    let mut stream = bus
        .subscribe(EventFilter {
            workspace_id: Some(r.workspace),
            kinds: Some(HashSet::from([EventKind::ClaimExpired])),
            ..EventFilter::default()
        })
        .await
        .unwrap();

    assert_eq!(claim_reaper::sweep_once(&state).await, 1);

    match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
        Ok(Some(BusItem::Event(env))) => match env.event {
            Event::ClaimExpired {
                thread_id,
                member_id,
                ..
            } => {
                assert_eq!(thread_id, dead.id);
                assert_eq!(member_id, r.holder);
            }
            other => panic!("expected ClaimExpired, got {other:?}"),
        },
        other => panic!("no ClaimExpired on the bus: {other:?}"),
    }

    let freed = store.get_thread(dead.id).await.unwrap();
    assert_eq!(freed.assignee_id, None);
    assert_eq!(freed.assignment_expires_at, None);
    assert_eq!(freed.claim_lease_id, None, "the fencing token is gone");
    let kept = store.get_thread(alive.id).await.unwrap();
    assert_eq!(
        kept.assignee_id,
        Some(r.holder),
        "a live lease is not reaped"
    );
    assert_eq!(kept.claim_lease_id, alive.claim_lease_id);

    // The dead holder's late heartbeat is fenced off.
    assert!(store
        .renew_claim(dead.id, r.holder, dead.claim_lease_id.unwrap(), 60)
        .await
        .is_err());

    // Nothing left to reap, and nothing more on the bus.
    assert_eq!(claim_reaper::sweep_once(&state).await, 0);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), stream.next())
            .await
            .is_err(),
        "a lease is reported once"
    );
}

#[tokio::test]
async fn reaper_frees_a_lapsed_lease_sqlite() {
    let (store, search) = sqlite().await;
    reaper_frees_a_lapsed_lease_and_publishes_claim_expired(store, search).await;
}

#[tokio::test]
async fn reaper_frees_a_lapsed_lease_postgres() {
    let Some((_container, pool)) = common::postgres_pool().await else {
        return;
    };
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    reaper_frees_a_lapsed_lease_and_publishes_claim_expired(store, search).await;
}

/// A leased claim its holder never acknowledged is reported once, with the
/// claim left alone, and the owner hears about it; an acknowledged claim is
/// not reported.
/// A hung agent never reports usage; the tick that frees its lapsed claim
/// charges the time it worked and stops it once that passes `max_wall_secs`.
async fn reaper_stops_a_hung_claim_past_its_wall_budget(
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
) {
    const LEASE_SECS: i64 = 3;
    let dir = tempfile::tempdir().unwrap();
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        bus.clone(),
        search,
    );
    let r = room(store.as_ref()).await;
    new_thread(store.as_ref(), r.channel, "hung").await;
    let held = store
        .claim_next_thread(r.channel, r.holder, Some(LEASE_SECS))
        .await
        .unwrap()
        .unwrap();
    store
        .set_thread_budget(
            held.id,
            BudgetLimits {
                max_wall_secs: Some(1),
                ..BudgetLimits::default()
            },
        )
        .await
        .unwrap();
    store
        .acknowledge_claim(held.id, r.holder, held.claim_lease_id.unwrap())
        .await
        .unwrap();
    let mut stream = bus
        .subscribe(EventFilter {
            workspace_id: Some(r.workspace),
            kinds: Some(HashSet::from([
                EventKind::ClaimExpired,
                EventKind::ClaimFailed,
            ])),
            ..EventFilter::default()
        })
        .await
        .unwrap();
    // The agent hangs: no report, no heartbeat, until the lease lapses.
    tokio::time::sleep(Duration::from_millis(LEASE_SECS as u64 * 1000 + 300)).await;

    assert_eq!(claim_reaper::sweep_once(&state).await, 1);
    match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
        Ok(Some(BusItem::Event(env))) => match env.event {
            Event::ClaimFailed {
                thread_id,
                member_id,
                reason,
                ..
            } => {
                assert_eq!(thread_id, held.id);
                assert_eq!(member_id, r.holder);
                assert_eq!(reason, "wall");
            }
            other => panic!("expected ClaimFailed, got {other:?}"),
        },
        other => panic!("no ClaimFailed on the bus: {other:?}"),
    }
    let dlq = store.list_channel_dlq(r.channel, 10).await.unwrap();
    assert_eq!(dlq.len(), 1);
    assert_eq!(dlq[0].reason, "wall");
    let budget = store.get_thread_budget(held.id).await.unwrap().unwrap();
    assert!(budget.used_wall_secs >= 1, "charged: {budget:?}");
    assert_eq!(store.get_thread(held.id).await.unwrap().assignee_id, None);
}

#[tokio::test]
async fn reaper_stops_a_hung_claim_past_its_wall_budget_sqlite() {
    let (store, search) = sqlite().await;
    reaper_stops_a_hung_claim_past_its_wall_budget(store, search).await;
}

#[tokio::test]
async fn reaper_stops_a_hung_claim_past_its_wall_budget_postgres() {
    let Some((_container, pool)) = common::postgres_pool().await else {
        return;
    };
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    reaper_stops_a_hung_claim_past_its_wall_budget(store, search).await;
}

async fn reaper_reports_an_unacknowledged_claim_once(
    store: Arc<dyn Store>,
    search: Arc<dyn maidan_search::Search>,
) {
    let dir = tempfile::tempdir().unwrap();
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let state = AppState::for_tests(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        bus.clone(),
        search,
    );
    let r = room(store.as_ref()).await;
    let owner = store
        .create_member(NewMember {
            workspace_id: r.workspace,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    new_thread(store.as_ref(), r.channel, "silent").await;
    new_thread(store.as_ref(), r.channel, "working").await;
    let silent = store
        .claim_next_thread(r.channel, r.holder, Some(3600))
        .await
        .unwrap()
        .unwrap();
    store
        .set_thread_owner(silent.id, Some(owner.id))
        .await
        .unwrap();
    let working = store
        .claim_next_thread(r.channel, r.holder, Some(3600))
        .await
        .unwrap()
        .unwrap();
    store
        .acknowledge_claim(working.id, r.holder, working.claim_lease_id.unwrap())
        .await
        .unwrap();

    let mut stream = bus
        .subscribe(EventFilter {
            workspace_id: Some(r.workspace),
            kinds: Some(HashSet::from([EventKind::ClaimUnacknowledged])),
            ..EventFilter::default()
        })
        .await
        .unwrap();

    // A zero window: every claim taken so far is past it.
    assert_eq!(
        claim_reaper::report_unacknowledged_once(&state, Duration::ZERO).await,
        1
    );
    let (log_id, event) = match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
        Ok(Some(BusItem::Event(env))) => (env.log_id, env.event),
        other => panic!("no ClaimUnacknowledged on the bus: {other:?}"),
    };
    match &event {
        Event::ClaimUnacknowledged {
            thread_id,
            member_id,
            claimed_at,
            ..
        } => {
            assert_eq!(*thread_id, silent.id);
            assert_eq!(*member_id, r.holder);
            assert!(*claimed_at <= chrono::Utc::now());
        }
        other => panic!("expected ClaimUnacknowledged, got {other:?}"),
    }
    let kept = store.get_thread(silent.id).await.unwrap();
    assert_eq!(kept.assignee_id, Some(r.holder), "the claim is left alone");
    assert_eq!(kept.claim_lease_id, silent.claim_lease_id);

    // Reported once.
    assert_eq!(
        claim_reaper::report_unacknowledged_once(&state, Duration::ZERO).await,
        0
    );

    // The owner is told the work is stuck.
    notification_router::route_event(&state, log_id, &event)
        .await
        .unwrap();
    let notes = store.list_notifications(owner.id, false, 10).await.unwrap();
    assert_eq!(notes.len(), 1, "the owner is notified");
    assert_eq!(notes[0].kind, EventKind::ClaimUnacknowledged);
    assert_eq!(notes[0].thread_id, Some(silent.id));
}

#[tokio::test]
async fn reaper_reports_an_unacknowledged_claim_once_sqlite() {
    let (store, search) = sqlite().await;
    reaper_reports_an_unacknowledged_claim_once(store, search).await;
}

#[tokio::test]
async fn reaper_reports_an_unacknowledged_claim_once_postgres() {
    let Some((_container, pool)) = common::postgres_pool().await else {
        return;
    };
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    reaper_reports_an_unacknowledged_claim_once(store, search).await;
}

struct Http {
    addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

async fn serve(store: Arc<dyn Store>, search: Arc<dyn maidan_search::Search>) -> Http {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    Http {
        addr,
        _server: server,
        _dir: dir,
    }
}

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn rest_claims_are_leased_by_default_and_leases_are_bounded() {
    let (store, search) = sqlite().await;
    let r = room(store.as_ref()).await;
    for title in ["one", "two"] {
        new_thread(store.as_ref(), r.channel, title).await;
    }
    let http = serve(store.clone(), search).await;
    let token = mint(store.as_ref(), r.workspace, r.holder).await;
    let client = reqwest::Client::new();
    let claim_next = format!(
        "http://{}/channels/{}/threads/claim-next",
        http.addr, r.channel.0
    );
    let post = |url: String, body: Value| {
        let client = client.clone();
        let token = token.clone();
        async move {
            let res = client
                .post(url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .unwrap();
            let status = res.status();
            (status, res.json::<Value>().await.unwrap_or(Value::Null))
        }
    };

    // Out of bounds: a 400 problem, and nothing is claimed.
    for bad in [0, -30, 7 * 24 * 60 * 60 + 1] {
        let (status, body) = post(claim_next.clone(), json!({ "lease_secs": bad })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "lease {bad}: {body}");
        assert!(
            body["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("lease_secs"),
            "{body}"
        );
    }
    assert!(store
        .list_threads(r.channel)
        .await
        .unwrap()
        .iter()
        .all(|t| t.assignee_id.is_none()));

    // No lease named: leased for the default 600 s, and fenced.
    let before = chrono::Utc::now();
    let (status, claimed) = post(claim_next.clone(), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let expires: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(claimed["assignment_expires_at"].clone()).unwrap();
    let lease = (expires - before).num_seconds();
    assert!((599..=601).contains(&lease), "default lease was {lease} s");
    assert!(claimed["claim_lease_id"].is_string());

    // Renewals are held to the same bounds; a valid one extends the lease.
    let renew = format!(
        "http://{}/threads/{}/claim/renew",
        http.addr,
        claimed["id"].as_str().unwrap()
    );
    let lease_id = claimed["claim_lease_id"].clone();
    let (status, body) = post(
        renew.clone(),
        json!({ "claim_lease_id": lease_id, "lease_secs": 0 }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = post(
        renew,
        json!({ "claim_lease_id": lease_id, "lease_secs": 1200 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let renewed: chrono::DateTime<chrono::Utc> =
        serde_json::from_value(body["assignment_expires_at"].clone()).unwrap();
    assert!(renewed > expires);
}
