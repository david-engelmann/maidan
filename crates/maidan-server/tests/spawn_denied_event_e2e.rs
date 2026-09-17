//! A spawn the budget refuses is observable. The caller still gets a 409, and
//! the room records a `ThreadSpawnDenied` naming the axis, the cap, what the
//! claim already held, and **who** tried — the fact the store's gate can't know
//! on the thread-create path.
//!
//! Auth is ENABLED with a real minted token: under bypass auth `auth.member_id`
//! is the nil member, so attribution is exactly what a bypass run cannot prove.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    Event, EventFilter, EventKind, MemberKind, NewApiToken, NewChannel, NewMember, NewThread,
    NewWorkspace,
};
use maidan_types::{MemberId, WorkspaceId};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

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
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

#[tokio::test]
async fn a_refused_spawn_publishes_thread_spawn_denied() {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let mut subscriber = bus
        .subscribe(EventFilter::all().with_kinds([EventKind::ThreadSpawnDenied]))
        .await
        .unwrap();

    let state = AppState::new(
        store.clone(),
        artifacts,
        bus.clone(),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED — the event must carry the real caller
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "spawn".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "runaway".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "q".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let parent = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parent claim".into()),
        })
        .await
        .unwrap();
    let token = mint(store.as_ref(), ws.id, member.id).await;

    // One child per parent. Set through the store, not the config route — the
    // subject here is the denial event, not the REST surface.
    store
        .set_spawn_budget(ws.id, Some(1), None, None)
        .await
        .unwrap();

    let spawn = || {
        let (client, base, token) = (client.clone(), base.clone(), token.clone());
        let (cid, pid) = (channel.id.0, parent.id.0);
        async move {
            client
                .post(format!("{base}/channels/{cid}/threads"))
                .bearer_auth(token)
                .json(&json!({"title": "helper", "parent_thread_id": pid}))
                .send()
                .await
                .unwrap()
        }
    };
    assert_eq!(
        spawn().await.status(),
        StatusCode::CREATED,
        "the first helper is in budget"
    );
    assert_eq!(
        spawn().await.status(),
        StatusCode::CONFLICT,
        "the second is refused"
    );

    let event = tokio::time::timeout(Duration::from_secs(2), subscriber.next())
        .await
        .expect("timeout waiting for ThreadSpawnDenied")
        .expect("subscriber ended without event");
    let BusItem::Event(envelope) = event else {
        panic!("expected event, got lag or end");
    };
    let log_id = envelope.log_id;
    match envelope.event {
        Event::ThreadSpawnDenied {
            workspace_id,
            channel_id,
            thread_id,
            member_id,
            axis,
            limit,
            observed,
            ..
        } => {
            assert_eq!(workspace_id, ws.id);
            assert_eq!(channel_id, channel.id);
            assert_eq!(thread_id, parent.id, "the capped parent");
            assert_eq!(
                member_id,
                Some(member.id),
                "the caller is who tried to spawn"
            );
            assert_eq!(axis, "children");
            assert_eq!(limit, 1);
            assert_eq!(observed, 1);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    // Durable too, not just a live notification.
    let stored = store.get_stored_event(log_id).await.expect("event logged");
    assert_eq!(stored.kind, EventKind::ThreadSpawnDenied);
    assert_eq!(stored.payload["axis"], "children");
    assert_eq!(stored.payload["member_id"], json!(member.id.0));

    // An accepted spawn denies nothing: clearing the cap re-opens spawning and
    // must not produce a second denial.
    store
        .set_spawn_budget(ws.id, None, None, None)
        .await
        .unwrap();
    assert_eq!(
        spawn().await.status(),
        StatusCode::CREATED,
        "a cleared budget re-opens spawning"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(200), subscriber.next())
            .await
            .is_err(),
        "an accepted spawn must not be denied"
    );
}
