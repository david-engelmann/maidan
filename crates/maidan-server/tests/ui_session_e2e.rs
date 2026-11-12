//! The `/ui/api/me` capability-card source: the console's Preferences card
//! reads the caller's capabilities from `GET /ui/api/me` and renders
//! `{can, can't}` = granted vs `known_capabilities − granted`. A bearer, and
//! a session made from a token, report that credential's grant. An OIDC
//! browser session reports what the console can do with it (the write routes
//! as well as the read routes), not only the read router this GET sits on. A
//! viewer invite stays on the read set. The DOM render is covered by
//! `ui-tests/tests/console-signin.spec.ts`.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AcceptWorkspaceInvite, AuditScope, MemberId, MemberKind, NewApiToken, NewAuditEvent,
    NewMaidanSession, NewMember, NewWorkspace, NewWorkspaceInvite, WorkspaceId,
    WorkspaceInviteRole,
};
use serde_json::{json, Value};

const SESSION_SECRET: &[u8] = b"ui-session-e2e-session-secret-32b!!";

async fn mint(
    store: &dyn Store,
    ws: WorkspaceId,
    member: MemberId,
    caps: Vec<String>,
) -> (maidan_types::ApiTokenId, String) {
    let secret = TokenSecret::generate();
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    (token.id, secret.as_str().to_string())
}

fn cookie_for(session_id: maidan_types::SessionId) -> String {
    let mut headers = axum::http::HeaderMap::new();
    maidan_server::session::set_session_cookie(
        &mut headers,
        session_id,
        3600,
        false,
        SESSION_SECRET,
    )
    .unwrap();
    headers
        .get(axum::http::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

fn strings(body: &Value, key: &str) -> Vec<String> {
    body[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.sessions = Some(maidan_server::session::SessionSettings {
        secret: Arc::from(SESSION_SECRET),
        ttl_secs: 3600,
        cookie_secure: false,
    });
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

#[tokio::test]
async fn ui_api_me_reports_granted_and_withheld_capabilities() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "ui".into() })
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
    // A deliberately narrow grant: read-only, so `workspace:write` lands in "can't".
    let (_id, token) = mint(
        store.as_ref(),
        ws.id,
        member.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;

    let me: Value = client
        .get(format!("{base}/ui/api/me"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(me["member_id"], Value::from(member.id.0.to_string()));
    assert_eq!(me["workspace_id"], Value::from(ws.id.0.to_string()));
    assert_eq!(me["is_bearer"], Value::Bool(true));

    let granted: Vec<String> = serde_json::from_value(me["capabilities"].clone()).unwrap();
    assert_eq!(granted, vec![capability::WORKSPACE_READ.to_string()]);

    let known: Vec<String> = serde_json::from_value(me["known_capabilities"].clone()).unwrap();
    // The full vocabulary is present so the card can compute "can't" = known − granted.
    assert!(known.contains(&capability::WORKSPACE_READ.to_string()));
    assert!(known.contains(&capability::WORKSPACE_WRITE.to_string()));
    assert!(known.contains(&capability::CHANNEL_ADMIN.to_string()));
    // "can't" is non-empty for a narrow grant — the whole point of the card.
    let cant: Vec<&String> = known.iter().filter(|c| !granted.contains(c)).collect();
    assert!(cant
        .iter()
        .any(|c| c.as_str() == capability::WORKSPACE_WRITE));
    assert!(cant.len() >= known.len() - 1);
}

#[tokio::test]
async fn an_oidc_session_reports_what_the_console_can_do() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "ui".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "ada".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let session = store
        .create_session(NewMaidanSession {
            workspace_id: ws.id,
            member_id: member.id,
            api_token_id: None,
            oidc_identity_id: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();

    let me: Value = client
        .get(format!("{base}/ui/api/me"))
        .header(reqwest::header::COOKIE, cookie_for(session.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(me["is_bearer"], Value::Bool(false));
    assert!(me["token_id"].is_null());
    let mut granted = strings(&me, "capabilities");
    granted.sort();
    let mut expect: Vec<String> = maidan_server::auth::OIDC_WRITE_CAPABILITIES
        .iter()
        .map(|c| (*c).to_string())
        .collect();
    expect.sort();
    assert_eq!(
        granted, expect,
        "posting and moving tasks are part of a signed-in session"
    );
    let known = strings(&me, "known_capabilities");
    for withheld in [
        capability::FEDERATION_ADMIN,
        capability::OPERATOR_GLOBAL,
        capability::TOKEN_ADMIN,
    ] {
        assert!(
            !granted.iter().any(|c| c == withheld),
            "a session does not hold {withheld}"
        );
        assert!(
            known.iter().any(|c| c == withheld),
            "the card can still list {withheld} under can't"
        );
    }
    // The console set is the worker recipe, so the named set is held. It is
    // not a minted token, and it is not the admin set.
    assert_eq!(me["capability_sets"], json!([capability::AGENT_WORKER]));
}

#[tokio::test]
async fn a_token_session_reports_the_tokens_grant_only() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "ui".into() })
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
    let (token_id, _secret) = mint(
        store.as_ref(),
        ws.id,
        member.id,
        vec![capability::WORKSPACE_READ.into()],
    )
    .await;
    let session = store
        .create_session(NewMaidanSession {
            workspace_id: ws.id,
            member_id: member.id,
            api_token_id: Some(token_id),
            oidc_identity_id: None,
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        })
        .await
        .unwrap();

    let me: Value = client
        .get(format!("{base}/ui/api/me"))
        .header(reqwest::header::COOKIE, cookie_for(session.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(me["is_bearer"], Value::Bool(true));
    assert_eq!(me["token_id"], Value::from(token_id.0.to_string()));
    assert_eq!(
        strings(&me, "capabilities"),
        vec![capability::WORKSPACE_READ.to_string()]
    );
    assert!(me["capability_sets"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_viewer_session_stays_on_the_read_set() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "ui".into() })
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
    let invite = store
        .create_workspace_invite_audited(
            NewWorkspaceInvite {
                workspace_id: ws.id,
                email: "vi@example.com".into(),
                role: WorkspaceInviteRole::Viewer,
                created_by: admin.id,
                token_hash: hash_secret("viewer-invite"),
                expires_at: chrono::Utc::now() + chrono::Duration::days(7),
            },
            Box::new(|invite| NewAuditEvent {
                scope: AuditScope::Workspace(invite.workspace_id),
                actor_id: Some(invite.created_by),
                action: "workspace_invite.create".into(),
                target_kind: Some("workspace_invite".into()),
                target_id: Some(invite.id.0),
                metadata: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();
    let accepted = store
        .accept_workspace_invite(
            AcceptWorkspaceInvite {
                invite_id: invite.id,
                workspace_id: ws.id,
                email: "vi@example.com".into(),
                issuer: "https://idp.example".into(),
                subject: "viewer-sub".into(),
                handle: "viewer".into(),
                session_expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            },
            Box::new(|accepted| NewAuditEvent {
                scope: AuditScope::Workspace(accepted.invite.workspace_id),
                actor_id: Some(accepted.member.id),
                action: "workspace_invite.accept".into(),
                target_kind: Some("workspace_invite".into()),
                target_id: Some(accepted.invite.id.0),
                metadata: serde_json::json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();

    let me: Value = client
        .get(format!("{base}/ui/api/me"))
        .header(reqwest::header::COOKIE, cookie_for(accepted.session.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(me["is_bearer"], Value::Bool(false));
    let mut granted = strings(&me, "capabilities");
    granted.sort();
    let mut expect: Vec<String> = maidan_server::auth::OIDC_READ_CAPABILITIES
        .iter()
        .map(|c| (*c).to_string())
        .collect();
    expect.sort();
    assert_eq!(granted, expect, "a viewer cannot post or move tasks");
    for absent in [
        capability::MESSAGE_POST,
        capability::THREAD_TRANSITION,
        capability::ARTIFACT_UPLOAD,
        capability::WORKSPACE_WRITE,
        capability::FEDERATION_ADMIN,
        capability::OPERATOR_GLOBAL,
    ] {
        assert!(!granted.iter().any(|c| c == absent), "{absent}");
    }
    assert!(me["capability_sets"].as_array().unwrap().is_empty());
}
