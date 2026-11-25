//! The waiting-on-you inbox lists a member's assigned threads only while the
//! member can still open them. Being assigned a thread in a private channel
//! does not keep the channel open to a member who has been removed from it, on
//! REST and on MCP alike; a member still in the channel, and a thread in a
//! public channel, are unaffected.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::test_support::{self, Caller, Tenant};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, ChannelMemberRole, MemberId, MemberKind, NewChannel, NewThread, ThreadId,
};
use serde_json::{json, Value};

struct Rig {
    base: String,
    store: Arc<dyn Store>,
    tenant: Tenant,
}

async fn rig() -> Rig {
    let pool = maidan_search::sqlite_pool_options()
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
    let tenant = test_support::tenant(store.as_ref(), "waiting-ws").await;
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Rig {
        base: format!("http://{addr}"),
        store,
        tenant,
    }
}

impl Rig {
    async fn member(&self, handle: &str) -> Caller {
        self.tenant
            .member(self.store.as_ref(), handle, MemberKind::Human)
            .await
    }

    async fn channel(&self, name: &str, private: bool) -> ChannelId {
        self.store
            .create_channel(NewChannel {
                workspace_id: self.tenant.workspace_id,
                name: name.into(),
                topic: None,
                private,
            })
            .await
            .unwrap()
            .id
    }

    async fn assigned(&self, channel_id: ChannelId, to: MemberId, title: &str) -> ThreadId {
        let thread = self
            .store
            .create_thread(NewThread {
                channel_id,
                parent_thread_id: None,
                title: Some(title.into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
        self.store.assign_thread(thread.id, to).await.unwrap();
        thread.id
    }

    /// The member's waiting inbox over REST, as text.
    async fn rest(&self, who: &Caller) -> String {
        let response = who
            .client()
            .get(format!("{}/members/{}/waiting", self.base, who.member_id.0))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success(), "{}", response.status());
        response.json::<Value>().await.unwrap().to_string()
    }

    /// The member's waiting inbox over MCP, as text.
    async fn mcp(&self, who: &Caller) -> String {
        let response: Value = who
            .client()
            .post(format!("{}/mcp", self.base))
            .json(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {
                    "name": "get_waiting_inbox",
                    "arguments": { "member_id": who.member_id.0 },
                },
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(response.get("error").is_none(), "{response}");
        assert_ne!(response["result"]["isError"], json!(true), "{response}");
        response.to_string()
    }
}

#[tokio::test]
async fn an_assigned_thread_in_a_private_channel_leaves_the_waiting_inbox_when_its_member_is_removed(
) {
    let rig = rig().await;
    let stayer = rig.member("stayer").await;
    let leaver = rig.member("leaver").await;
    let private = rig.channel("incident", true).await;
    let public = rig.channel("general", false).await;
    for member in [stayer.member_id, leaver.member_id] {
        rig.store
            .add_channel_member(private, member, ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    rig.assigned(private, stayer.member_id, "stayer private job")
        .await;
    rig.assigned(private, leaver.member_id, "leaver private job")
        .await;
    rig.assigned(public, leaver.member_id, "leaver public job")
        .await;

    // Both are in the channel: each sees their own threads.
    for inbox in [rig.rest(&leaver).await, rig.mcp(&leaver).await] {
        assert!(inbox.contains("leaver private job"), "{inbox}");
        assert!(inbox.contains("leaver public job"), "{inbox}");
    }
    for inbox in [rig.rest(&stayer).await, rig.mcp(&stayer).await] {
        assert!(inbox.contains("stayer private job"), "{inbox}");
    }

    rig.store
        .remove_channel_member(private, leaver.member_id)
        .await
        .unwrap();

    // The leaver keeps what they can still open and loses what they cannot,
    // on both surfaces; the member still in the channel keeps theirs.
    for (surface, inbox) in [
        ("REST", rig.rest(&leaver).await),
        ("MCP", rig.mcp(&leaver).await),
    ] {
        assert!(
            !inbox.contains("leaver private job"),
            "{surface} still lists a thread in a channel the member left: {inbox}"
        );
        assert!(
            inbox.contains("leaver public job"),
            "{surface} dropped a thread the member can still open: {inbox}"
        );
    }
    for (surface, inbox) in [
        ("REST", rig.rest(&stayer).await),
        ("MCP", rig.mcp(&stayer).await),
    ] {
        assert!(
            inbox.contains("stayer private job"),
            "{surface} dropped a thread of a member still in the channel: {inbox}"
        );
    }
}
