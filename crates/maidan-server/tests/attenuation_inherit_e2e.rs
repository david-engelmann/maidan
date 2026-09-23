//! A derived token inherits every limit the parent carried.
//!
//! `attenuate` deliberately permits an equal capability list — a no-op re-issue
//! is a legitimate way to get a fresh secret. That makes any bound the parent
//! carried and the child did not into a way of shedding that bound by asking.
//! Two were being dropped: the app installation, and the per-token quotas.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use chrono::{Duration as ChronoDuration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewApp, NewAppInstallation, NewDelegationGrant, NewMember,
    NewWorkspace, TokenQuota,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    std::mem::forget(dir);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

/// The headline: an installed app derives a token, the workspace revokes the
/// installation, and the derived token must stop working. Before 397.7 the
/// derived token named no installation, so the revoke had nothing to check and
/// the app kept its access after being uninstalled.
#[tokio::test]
async fn a_derived_token_dies_with_the_app_installation() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let app = store
        .create_app(NewApp {
            workspace_id: ws.id,
            slug: "an-app".into(),
            name: "An App".into(),
            description: None,
            created_by: bot.id,
        })
        .await
        .unwrap();
    let install = store
        .create_app_installation(NewAppInstallation {
            app_id: app.id,
            workspace_id: ws.id,
            bot_member_id: bot.id,
            granted_capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
        })
        .await
        .unwrap();

    // The app's own bearer, tied to the installation.
    let app_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: bot.id,
            app_installation_id: Some(install.id),
            token_hash: hash_secret(app_secret.as_str()),
            label: Some("app".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();

    let derived: Value = client
        .post(format!("{base}/tokens/attenuate"))
        .bearer_auth(app_secret.as_str())
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ], "label": "derived" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let derived_secret = derived["secret"].as_str().expect("minted").to_string();

    // It works while the installation stands.
    let before = client
        .get(format!("{base}/workspaces/{}", ws.id.0))
        .bearer_auth(&derived_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK, "derived token works");

    // Uninstall.
    store.revoke_app_installation(install.id).await.unwrap();

    let after = client
        .get(format!("{base}/workspaces/{}", ws.id.0))
        .bearer_auth(&derived_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(
        after.status(),
        StatusCode::UNAUTHORIZED,
        "a derived token must not outlive the installation it came from"
    );
}

/// Quotas are keyed on the token id, so a child with none is a child with no
/// throttle — reachable by re-issuing the same capability list.
#[tokio::test]
async fn a_derived_token_inherits_the_parents_quotas() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
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
    let secret = TokenSecret::generate();
    let parent = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("throttled".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let quota = TokenQuota {
        capability: capability::MESSAGE_POST.into(),
        max_per_window: 10,
        window_secs: 60,
    };
    store
        .replace_token_quotas(parent.id, std::slice::from_ref(&quota))
        .await
        .unwrap();

    // A no-op re-issue: the same capabilities, a fresh secret.
    let derived: Value = client
        .post(format!("{base}/tokens/attenuate"))
        .bearer_auth(secret.as_str())
        .json(&json!({
            "capabilities": [capability::WORKSPACE_READ, capability::MESSAGE_POST]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let derived_id =
        maidan_types::ApiTokenId(uuid::Uuid::parse_str(derived["id"].as_str().unwrap()).unwrap());

    let inherited = store.list_token_quotas(derived_id).await.unwrap();
    assert_eq!(
        inherited,
        vec![quota],
        "a re-issue must not shed the parent's throttle"
    );
    // And it is reported back, so the holder can see what it got.
    assert_eq!(derived["quotas"][0]["capability"], capability::MESSAGE_POST);
}

#[tokio::test]
async fn delegated_tokens_are_bounded_and_die_with_the_grant() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let subject = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "subject".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let delegate = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "delegate".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let other = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "other".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let grant = store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: subject.id,
            delegate_id: delegate.id,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::MESSAGE_POST.into(),
            ],
            purpose: "cover incident".into(),
            authorized_by: subject.id,
            expires_at: Utc::now() + ChronoDuration::hours(2),
        })
        .await
        .unwrap();

    let mint_delegate = async |member_id| {
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities: vec![capability::WORKSPACE_READ.into()],
                expires_at: None,
            })
            .await
            .unwrap();
        secret
    };
    let delegate_secret = mint_delegate(delegate.id).await;
    let other_secret = mint_delegate(other.id).await;

    let wrong = client
        .post(format!("{base}/tokens/delegate"))
        .bearer_auth(other_secret.as_str())
        .json(&json!({ "grant_id": grant.id.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);

    let too_long = client
        .post(format!("{base}/tokens/delegate"))
        .bearer_auth(delegate_secret.as_str())
        .json(&json!({
            "grant_id": grant.id.0,
            "expires_at": Utc::now() + ChronoDuration::hours(1) + ChronoDuration::minutes(1)
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(too_long.status(), StatusCode::BAD_REQUEST);

    let exchanged: Value = client
        .post(format!("{base}/tokens/delegate"))
        .bearer_auth(delegate_secret.as_str())
        .json(&json!({ "grant_id": grant.id.0 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(exchanged["token"]["member_id"], subject.id.0.to_string());
    assert_eq!(
        exchanged["token"]["capabilities"],
        json!([capability::WORKSPACE_READ])
    );
    let delegated_secret = exchanged["token"]["secret"].as_str().unwrap();

    let child: Value = client
        .post(format!("{base}/tokens/attenuate"))
        .bearer_auth(delegated_secret)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let child_secret = child["secret"].as_str().unwrap();
    assert_eq!(
        client
            .get(format!("{base}/workspaces/{}", ws.id.0))
            .bearer_auth(child_secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    assert!(store
        .revoke_delegation_grant(ws.id, grant.id)
        .await
        .unwrap());
    for secret in [delegated_secret, child_secret] {
        assert_eq!(
            client
                .get(format!("{base}/workspaces/{}", ws.id.0))
                .bearer_auth(secret)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn grant_admin_and_dual_identity_evidence_work_over_rest() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let admin = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let subject = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "subject".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let delegate = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "delegate".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();

    let mint = async |member_id, capabilities: Vec<String>| {
        let secret = TokenSecret::generate();
        store
            .create_api_token(NewApiToken {
                workspace_id: ws.id,
                member_id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: None,
                capabilities,
                expires_at: None,
            })
            .await
            .unwrap();
        secret
    };
    let admin_secret = mint(admin.id, vec![capability::TOKEN_ADMIN.into()]).await;
    let delegate_secret = mint(delegate.id, vec![capability::WORKSPACE_READ.into()]).await;

    let created = client
        .post(format!("{base}/workspaces/{}/delegation-grants", ws.id.0))
        .bearer_auth(admin_secret.as_str())
        .json(&json!({
            "subject_id": subject.id.0,
            "delegate_id": delegate.id.0,
            "capabilities": [capability::WORKSPACE_READ],
            "purpose": "cover the incident",
            "expires_at": Utc::now() + ChronoDuration::hours(1),
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let grant: Value = created.json().await.unwrap();
    let grant_id = grant["id"].as_str().unwrap();

    let listed: Vec<Value> = client
        .get(format!("{base}/workspaces/{}/delegation-grants", ws.id.0))
        .bearer_auth(admin_secret.as_str())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);

    let exchanged: Value = client
        .post(format!("{base}/tokens/delegate"))
        .bearer_auth(delegate_secret.as_str())
        .json(&json!({ "grant_id": grant_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let delegated_secret = exchanged["token"]["secret"].as_str().unwrap();
    let me: Value = client
        .get(format!("{base}/me"))
        .bearer_auth(delegated_secret)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(me["actor_id"], delegate.id.0.to_string());
    assert_eq!(me["member_id"], subject.id.0.to_string());
    assert_eq!(me["delegation_grant_id"], grant_id);

    let denied = client
        .get(format!("{base}/operator/audit"))
        .bearer_auth(delegated_secret)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let decisions: Vec<_> = store
        .list_audit_for_workspace(ws.id, 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|event| event.action == "authorization.decision")
        .collect();
    assert!(decisions.iter().any(|event| {
        event.actor_id == Some(delegate.id)
            && event.metadata["subject_id"] == json!(subject.id.0)
            && event.metadata["grant_id"] == json!(grant_id)
            && event.metadata["outcome"] == "allowed"
    }));
    assert!(decisions
        .iter()
        .any(|event| event.metadata["outcome"] == "denied"));

    let revoked = client
        .delete(format!(
            "{base}/workspaces/{}/delegation-grants/{grant_id}",
            ws.id.0
        ))
        .bearer_auth(admin_secret.as_str())
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::OK);
    assert!(revoked.json::<Value>().await.unwrap()["revoked_at"].is_string());
    assert_eq!(
        client
            .get(format!("{base}/me"))
            .bearer_auth(delegated_secret)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

/// Revoking a token kills everything derived from it, and the derived
/// credential actually stops working — not merely gets a column set.
///
/// The parent link used to live only in audit metadata, so revocation could not
/// traverse it and a child outlived the credential it was minted from. The same
/// shape was closed earlier for app installations and quotas; this is the third
/// dimension.
#[tokio::test]
async fn a_derived_token_dies_with_its_parent() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "revoke-e2e".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "rev-agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();

    let parent_secret = TokenSecret::generate();
    let parent = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(parent_secret.as_str()),
            label: Some("parent".into()),
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let attenuate = |bearer: String| {
        let (base, client) = (base.clone(), client.clone());
        async move {
            let v: Value = client
                .post(format!("{base}/tokens/attenuate"))
                .bearer_auth(bearer)
                .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            v["secret"].as_str().expect("minted").to_string()
        }
    };
    let child_secret = attenuate(parent_secret.as_str().to_string()).await;
    // Derived from the *child*, so this only survives a one-level cascade.
    let grandchild_secret = attenuate(child_secret.clone()).await;

    let reads = |bearer: String| {
        let (base, client, wsid) = (base.clone(), client.clone(), ws.id.0);
        async move {
            client
                .get(format!("{base}/workspaces/{wsid}"))
                .bearer_auth(bearer)
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(reads(child_secret.clone()).await, StatusCode::OK);
    assert_eq!(reads(grandchild_secret.clone()).await, StatusCode::OK);

    store.revoke_api_token(parent.id).await.unwrap();

    assert_eq!(
        reads(child_secret).await,
        StatusCode::UNAUTHORIZED,
        "a derived token must not outlive the credential it was minted from"
    );
    assert_eq!(
        reads(grandchild_secret).await,
        StatusCode::UNAUTHORIZED,
        "the cascade must be transitive — a one-level kill is the same leak \
         one generation down"
    );
}

/// Shared setup for the escalation tests: an admin, an orchestrator that itself
/// holds `token:admin` (so every refusal is about the *borrowed* token, not the
/// orchestrator lacking a capability), and two agents.
struct Escalation {
    base: String,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    ws: maidan_types::WorkspaceId,
    orchestrator: MemberId,
    worker: MemberId,
    third: MemberId,
    admin_tok: String,
    orch_tok: String,
}

async fn escalation_setup() -> Escalation {
    let (addr, client, store) = spawn().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let mut ids = Vec::new();
    for (handle, kind) in [
        ("admin", MemberKind::Human),
        ("orchestrator", MemberKind::Agent),
        ("worker", MemberKind::Agent),
        ("third", MemberKind::Agent),
    ] {
        ids.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let mint = |member_id: MemberId, caps: Vec<String>| {
        let store = store.clone();
        async move {
            let secret = TokenSecret::generate();
            store
                .create_api_token(NewApiToken {
                    workspace_id: ws.id,
                    member_id,
                    app_installation_id: None,
                    token_hash: hash_secret(secret.as_str()),
                    label: None,
                    capabilities: caps,
                    expires_at: None,
                })
                .await
                .unwrap();
            secret.as_str().to_string()
        }
    };
    let admin_tok = mint(ids[0], vec![capability::TOKEN_ADMIN.into()]).await;
    let orch_tok = mint(
        ids[1],
        vec![
            capability::WORKSPACE_READ.into(),
            capability::TOKEN_ADMIN.into(),
        ],
    )
    .await;
    Escalation {
        base: format!("http://{addr}"),
        client,
        store,
        ws: ws.id,
        orchestrator: ids[1],
        worker: ids[2],
        third: ids[3],
        admin_tok,
        orch_tok,
    }
}

impl Escalation {
    fn grants_url(&self) -> String {
        format!("{}/workspaces/{}/delegation-grants", self.base, self.ws.0)
    }

    async fn grant_over_rest(
        &self,
        subject: MemberId,
        delegate: MemberId,
        caps: &[&str],
    ) -> reqwest::Response {
        self.client
            .post(self.grants_url())
            .bearer_auth(&self.admin_tok)
            .json(&json!({
                "subject_id": subject.0,
                "delegate_id": delegate.0,
                "capabilities": caps,
                "purpose": "run the worker",
                "expires_at": Utc::now() + ChronoDuration::hours(1),
            }))
            .send()
            .await
            .unwrap()
    }

    async fn exchange(&self, bearer: &str, grant_id: &Value) -> reqwest::Response {
        self.client
            .post(format!("{}/tokens/delegate", self.base))
            .bearer_auth(bearer)
            .json(&json!({ "grant_id": grant_id }))
            .send()
            .await
            .unwrap()
    }

    /// A borrowed token for `worker`, held by the orchestrator, lent work only.
    async fn borrowed_worker_token(&self) -> String {
        let grant: Value = self
            .grant_over_rest(
                self.worker,
                self.orchestrator,
                &[capability::WORKSPACE_READ],
            )
            .await
            .json()
            .await
            .unwrap();
        let exchanged: Value = self
            .exchange(&self.orch_tok, &grant["id"])
            .await
            .json()
            .await
            .unwrap();
        exchanged["token"]["secret"].as_str().unwrap().to_string()
    }
}

/// A grant lends the ability to do work, never the means to hand out more.
#[tokio::test]
async fn a_grant_cannot_lend_authority() {
    let e = escalation_setup().await;
    for authority in [
        capability::TOKEN_ADMIN,
        capability::CHANNEL_ADMIN,
        capability::SECRET_READ,
        capability::OPERATOR_GLOBAL,
    ] {
        let refused = e
            .grant_over_rest(e.worker, e.orchestrator, &[authority])
            .await;
        assert_eq!(
            refused.status(),
            StatusCode::BAD_REQUEST,
            "a grant must not carry {authority}"
        );
    }
}

/// The route refuses such a grant, but a grant can also arrive through the store
/// — created before that guard existed, or by import. This one carries
/// `token:admin`, and the borrowed token must still not wield it: a token minted
/// under it would not descend from the grant and would outlive its revocation.
#[tokio::test]
async fn a_borrowed_context_never_holds_authority() {
    let e = escalation_setup().await;
    let grant = e
        .store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: e.ws,
            subject_id: e.worker,
            delegate_id: e.orchestrator,
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::TOKEN_ADMIN.into(),
            ],
            purpose: "pre-dates the guard".into(),
            authorized_by: e.orchestrator,
            expires_at: Utc::now() + ChronoDuration::hours(1),
        })
        .await
        .unwrap();
    let exchanged: Value = e
        .exchange(&e.orch_tok, &json!(grant.id.0))
        .await
        .json()
        .await
        .unwrap();
    let borrowed = exchanged["token"]["secret"].as_str().unwrap().to_string();

    let minted = e
        .client
        .post(format!(
            "{}/workspaces/{}/members/{}/tokens",
            e.base, e.ws.0, e.worker.0
        ))
        .bearer_auth(&borrowed)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        minted.status(),
        StatusCode::FORBIDDEN,
        "a borrowed token must not mint standing tokens, whatever its grant says"
    );
}

/// Delegation is one hop: a borrowed token cannot exchange any grant at all.
///
/// Two cases, because two protections are involved. The first is the one only
/// the one-hop rule stops: the orchestrator's *own* second grant, where the
/// delegate check passes — the delegate really is the orchestrator — but the
/// token presenting it is borrowed, so the new token's lineage would run through
/// a borrowed one. The second is the chain: the worker's grant to act as
/// `third`, which would make the orchestrator `third` while every record named
/// the worker as the one who did it.
#[tokio::test]
async fn delegation_is_one_hop() {
    let e = escalation_setup().await;
    let borrowed = e.borrowed_worker_token().await;

    let orchestrators_own: Value = e
        .grant_over_rest(e.third, e.orchestrator, &[capability::WORKSPACE_READ])
        .await
        .json()
        .await
        .unwrap();
    let from_borrowed = e.exchange(&borrowed, &orchestrators_own["id"]).await;
    assert_eq!(
        from_borrowed.status(),
        StatusCode::FORBIDDEN,
        "a borrowed token must not exchange even its own delegate's grant"
    );

    let worker_to_third: Value = e
        .grant_over_rest(e.third, e.worker, &[capability::WORKSPACE_READ])
        .await
        .json()
        .await
        .unwrap();
    let chained = e.exchange(&borrowed, &worker_to_third["id"]).await;
    assert_eq!(
        chained.status(),
        StatusCode::FORBIDDEN,
        "a borrowed token must not chain into its subject's grant"
    );
}

/// A narrowed child of a borrowed token stays borrowed: the store copies the
/// parent's grant onto it. This already worked and had no test. If it stopped —
/// the child coming out as an ordinary token — every later use would read as the
/// worker acting alone, and the record of who is really acting would be shed.
#[tokio::test]
async fn a_narrowed_borrowed_token_stays_borrowed() {
    let e = escalation_setup().await;
    let borrowed = e.borrowed_worker_token().await;
    let resp = e
        .client
        .post(format!("{}/tokens/attenuate", e.base))
        .bearer_auth(&borrowed)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "narrowing must be allowed"
    );
    let child: Value = resp.json().await.unwrap();
    let child = child["secret"].as_str().unwrap().to_string();

    let me: Value = e
        .client
        .get(format!("{}/me", e.base))
        .bearer_auth(&child)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        me["actor_id"],
        e.orchestrator.0.to_string(),
        "the child must still name the orchestrator as the one acting"
    );
    assert_eq!(me["member_id"], e.worker.0.to_string());
    assert!(
        !me["delegation_grant_id"].is_null(),
        "the child must still carry the grant it was borrowed under"
    );
}

/// Another workspace's grant reads exactly like one that does not exist, as it
/// already did on revoke. A 403 here would confirm the id is real somewhere.
#[tokio::test]
async fn another_workspaces_grant_is_indistinguishable_from_none() {
    let e = escalation_setup().await;
    let other_ws = e
        .store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap();
    let mut others = Vec::new();
    for handle in ["s", "d"] {
        others.push(
            e.store
                .create_member(NewMember {
                    workspace_id: other_ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
                .id,
        );
    }
    let foreign = e
        .store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: other_ws.id,
            subject_id: others[0],
            delegate_id: others[1],
            capabilities: vec![capability::WORKSPACE_READ.into()],
            purpose: "elsewhere".into(),
            authorized_by: others[0],
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();

    let foreign = e.exchange(&e.orch_tok, &json!(foreign.id.0)).await;
    let missing = e.exchange(&e.orch_tok, &json!(uuid::Uuid::new_v4())).await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        foreign.status(),
        missing.status(),
        "another workspace's grant must answer like a missing one"
    );
}
