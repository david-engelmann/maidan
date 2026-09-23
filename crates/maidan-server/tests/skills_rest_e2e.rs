//! Capability-registry REST: declare/list/remove member skills and thread
//! required-skills. Auth ENABLED (real token) so RBAC is exercised.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId) -> String {
    mint_with(
        store,
        ws,
        member,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
            capability::THREAD_TRANSITION.into(),
        ],
    )
    .await
}

async fn mint_with(
    store: &dyn Store,
    ws: WorkspaceId,
    member: MemberId,
    capabilities: Vec<String>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn skills_crud_over_http() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "s".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let colleague = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "colleague".into(),
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
    let thread = store
        .create_thread(NewThread {
            channel_id: ChannelId(channel.id.0),
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap();
    let tok = mint(store.as_ref(), ws.id, member.id).await;
    let bearer = format!("Bearer {tok}");

    // --- member skills ---
    let add = client
        .post(format!("{base}/members/{}/skills", member.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "skill": "rust" }))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);

    let skills: Value = client
        .get(format!("{base}/members/{}/skills", member.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(skills.as_array().unwrap().len(), 1);
    assert_eq!(skills[0]["skill"], json!("rust"));

    let del = client
        .delete(format!("{base}/members/{}/skills/rust", member.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let del2 = client
        .delete(format!("{base}/members/{}/skills/rust", member.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del2.status(), StatusCode::NOT_FOUND);

    // Empty skill is rejected.
    let bad = client
        .post(format!("{base}/members/{}/skills", member.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "skill": "  " }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    for request in [
        client
            .get(format!("{base}/members/{}/skills", colleague.id.0))
            .header("Authorization", &bearer),
        client
            .post(format!("{base}/members/{}/skills", colleague.id.0))
            .header("Authorization", &bearer)
            .json(&json!({ "skill": "rust" })),
        client
            .delete(format!("{base}/members/{}/skills/rust", colleague.id.0))
            .header("Authorization", &bearer),
    ] {
        assert_eq!(
            request.send().await.unwrap().status(),
            StatusCode::FORBIDDEN,
            "ordinary tokens must not read or rewrite another member's skills"
        );
    }

    // --- thread required skills ---
    let add = client
        .post(format!("{base}/threads/{}/required-skills", thread.id.0))
        .header("Authorization", &bearer)
        .json(&json!({ "skill": "code-review" }))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), StatusCode::NO_CONTENT);

    let reqs: Value = client
        .get(format!("{base}/threads/{}/required-skills", thread.id.0))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reqs.as_array().unwrap().len(), 1);
    assert_eq!(reqs[0]["skill"], json!("code-review"));

    let del = client
        .delete(format!(
            "{base}/threads/{}/required-skills/code-review",
            thread.id.0
        ))
        .header("Authorization", &bearer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
}

/// A governance skill is not self-service.
///
/// The close-gate counts a green pass only from a member who
/// **declared** `land_gate`, and `request_changes` is armed only for a
/// producer who declared `review`. Granting was plain `workspace:write` with no
/// restriction on which skill — so an agent could grant *itself* the
/// qualification the gate exists to check, leaving separation of duties as the
/// only thing still standing.
///
/// Granting now ratchets like the gates: `channel:admin`, which
/// `maidan.agent.worker` does not carry.
#[tokio::test]
async fn an_agent_cannot_grant_itself_a_governance_skill() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "gov".into() })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "worker".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let worker_token = mint(&*store, ws.id, agent.id).await;

    let grant = |token: String, member: MemberId, skill: &'static str| {
        let (base, client) = (base.clone(), client.clone());
        async move {
            client
                .post(format!("{base}/members/{}/skills", member.0))
                .bearer_auth(token)
                .json(&json!({ "skill": skill }))
                .send()
                .await
                .unwrap()
                .status()
        }
    };

    // The escalation this closes: the agent naming itself.
    for skill in ["land_gate", "review"] {
        assert_eq!(
            grant(worker_token.clone(), agent.id, skill).await,
            StatusCode::FORBIDDEN,
            "{skill} must not be self-grantable on workspace:write"
        );
    }
    // Spelling is not a way around it — the grant surface stores what it is sent.
    assert_eq!(
        grant(worker_token.clone(), agent.id, "  LAND_GATE ").await,
        StatusCode::FORBIDDEN,
        "the gate must not be case- or whitespace-evadable"
    );

    // Ordinary routing tags stay self-service. The point is a narrow gate, not
    // a locked-down registry — `claim_next` routing must keep working.
    assert_eq!(
        grant(worker_token.clone(), agent.id, "rust").await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        store.list_member_skills(agent.id).await.unwrap().len(),
        1,
        "only the routing tag landed"
    );

    // An operator holding channel:admin can grant it — the capability is the
    // control, not a blanket ban.
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "human-admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let admin_token = mint_with(
        &*store,
        ws.id,
        admin.id,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::WORKSPACE_WRITE.into(),
            capability::CHANNEL_ADMIN.into(),
        ],
    )
    .await;
    assert_eq!(
        grant(admin_token.clone(), agent.id, "land_gate").await,
        StatusCode::NO_CONTENT,
        "channel:admin may widen who approves — on someone else, with no act-as-any"
    );
    let skills = store.list_member_skills(agent.id).await.unwrap();
    assert!(skills.iter().any(|s| s.skill == "land_gate"));

    // And take it away again, without the holder's cooperation. Revoking
    // approval authority from a misbehaving agent must not depend on the agent.
    let revoked = client
        .delete(format!("{base}/members/{}/skills/land_gate", agent.id.0))
        .bearer_auth(admin_token.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    assert!(!store
        .list_member_skills(agent.id)
        .await
        .unwrap()
        .iter()
        .any(|s| s.skill == "land_gate"));

    // Admin authority over governance is not authority over routing tags: those
    // stay the member's own.
    assert_eq!(
        grant(admin_token.clone(), agent.id, "python").await,
        StatusCode::FORBIDDEN,
        "a routing tag is personal state, even to an admin"
    );

    // And the grant is audited, because it is now a privileged operation.
    let audit = store
        .list_audit_for_workspace(ws.id, 50)
        .await
        .expect("audit");
    assert!(
        audit
            .iter()
            .any(|a| a.action == "member_skill.grant_governance"),
        "widening who may approve must leave a trace: {audit:?}"
    );
}
