//! The hard allowance cutoff: an open sign-up workspace past its free
//! allowance on a meter is refused that meter's work with a 429, while an
//! operator-created or invited workspace past the same allowance is only
//! marked. One workspace's usage never cuts off another.

mod seeded_workspace;

use std::time::Duration;

use chrono::Utc;
use maidan_auth::capability;
use maidan_server::AppState;
use maidan_store::Store;
use maidan_types::*;
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn_with, token, Harness};
use serde_json::Value;

async fn signed_up(store: &dyn Store, who: &str, open: bool) -> (WorkspaceId, MemberId) {
    let outcome = store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
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
    (outcome.workspace.id, outcome.member.id)
}

async fn status(h: &Harness, bearer: &str, path: &str) -> (StatusCode, Value) {
    let res = h.send(Method::GET, path, bearer, None).await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn admin_caps() -> Vec<String> {
    vec![
        capability::WORKSPACE_READ.to_string(),
        capability::TOKEN_ADMIN.to_string(),
    ]
}

#[tokio::test]
async fn an_open_signup_workspace_past_its_allowance_is_refused_while_an_operator_one_is_only_marked(
) {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = std::sync::Arc::new(UsageAllowance::parse("requests=2").unwrap());
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();

    let (open, open_admin) = signed_up(store, "ada", true).await;
    let open_token = token(store, open, open_admin, admin_caps()).await;
    let (other_open, other_admin) = signed_up(store, "bea", true).await;
    let other_token = token(store, other_open, other_admin, admin_caps()).await;
    let (invited, invited_admin) = signed_up(store, "cy", false).await;
    let invited_token = token(store, invited, invited_admin, admin_caps()).await;
    let ops = store
        .create_workspace(NewWorkspace { name: "ops".into() })
        .await
        .unwrap()
        .id;
    let ops_admin = member(store, ops, "admin", MemberKind::Human).await;
    let ops_token = token(store, ops, ops_admin, admin_caps()).await;
    let home = |ws: WorkspaceId| format!("/workspaces/{}", ws.0);

    // The open workspace, the invited one and the operator's each go past
    // two requests; the other open workspace makes one.
    for _ in 0..3 {
        for (bearer, ws) in [
            (&open_token, open),
            (&invited_token, invited),
            (&ops_token, ops),
        ] {
            assert_eq!(status(&h, bearer, &home(ws)).await.0, StatusCode::OK);
        }
    }
    assert_eq!(
        status(&h, &other_token, &home(other_open)).await.0,
        StatusCode::OK
    );

    let (_, _, marked) = maidan_server::usage_meters::tick(&state).await;
    assert_eq!(marked, 3, "all three past it are marked");

    // The open workspace is refused, and told why.
    let (code, body) = status(&h, &open_token, &home(open)).await;
    assert_eq!(code, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(
        body.to_string().contains("free allowance for requests"),
        "{body}"
    );
    let told = store
        .list_notifications(open_admin, false, 10)
        .await
        .unwrap();
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].kind, EventKind::UsageOverAllowance);
    // Its admin may still read its usage and its notifications.
    let (code, usage) = status(
        &h,
        &open_token,
        &format!("/workspaces/{}/usage-meters", open.0),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "{usage}");
    let (code, _) = status(
        &h,
        &open_token,
        &format!("/members/{}/notifications", open_admin.0),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    // Refusals are not counted toward anything.
    let (_, _, marked) = maidan_server::usage_meters::tick(&state).await;
    assert_eq!(marked, 0);

    // The invited and operator workspaces are only marked.
    for (bearer, ws) in [(&invited_token, invited), (&ops_token, ops)] {
        assert_eq!(status(&h, bearer, &home(ws)).await.0, StatusCode::OK);
    }
    // The other open workspace is inside its own allowance: A's usage is
    // not B's.
    assert_eq!(
        status(&h, &other_token, &home(other_open)).await.0,
        StatusCode::OK
    );
    assert!(!state
        .usage_meters
        .is_cut_off(other_open, UsageMeter::Requests));
    // And the open workspace is cut off only on the meter it crossed.
    assert!(!state.usage_meters.is_cut_off(open, UsageMeter::McpCalls));
}

#[tokio::test]
async fn the_cutoff_lifts_when_the_operator_drops_the_allowance() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = std::sync::Arc::new(UsageAllowance::parse("requests=1").unwrap());
        state = Some(s.clone());
    })
    .await;
    let mut state = state.unwrap();
    let store = h.store.as_ref();
    let (open, admin) = signed_up(store, "ada", true).await;
    let bearer = token(store, open, admin, admin_caps()).await;
    let home = format!("/workspaces/{}", open.0);
    for _ in 0..2 {
        status(&h, &bearer, &home).await;
    }
    maidan_server::usage_meters::tick(&state).await;
    assert!(state.usage_meters.is_cut_off(open, UsageMeter::Requests));

    state.usage_allowance = std::sync::Arc::new(UsageAllowance::default());
    maidan_server::usage_meters::refresh_cut_off(&state).await;
    assert!(!state.usage_meters.is_cut_off(open, UsageMeter::Requests));
    assert_eq!(status(&h, &bearer, &home).await.0, StatusCode::OK);
}

async fn a_thread(store: &dyn Store, ws: WorkspaceId) -> ThreadId {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn past_its_stored_bytes_an_open_workspace_cannot_add_messages_while_others_can() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = std::sync::Arc::new(UsageAllowance::parse("stored_bytes=10").unwrap());
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let caps = || {
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::MESSAGE_POST.to_string(),
        ]
    };
    let (open, open_admin) = signed_up(store, "ada", true).await;
    let (other, other_admin) = signed_up(store, "bea", true).await;
    let (invited, invited_admin) = signed_up(store, "cy", false).await;
    let ops = store
        .create_workspace(NewWorkspace { name: "ops".into() })
        .await
        .unwrap()
        .id;
    let ops_admin = member(store, ops, "admin", MemberKind::Human).await;
    let month = UsageMonth::of(Utc::now().date_naive()).label();
    for ws in [open, invited, ops] {
        let mark = OverAllowance {
            workspace_id: ws,
            meter: UsageMeter::StoredBytes,
            month: month.clone(),
            quantity: 11,
            allowance: 10,
            marked_at: Utc::now(),
        };
        store.mark_over_allowance(&mark, &[]).await.unwrap();
    }
    maidan_server::usage_meters::refresh_cut_off(&state).await;

    let post = |bearer: String, thread: ThreadId| {
        let h = &h;
        async move {
            h.send(
                Method::POST,
                &format!("/threads/{}/messages", thread.0),
                &bearer,
                Some(serde_json::json!({ "body": "hello" })),
            )
            .await
        }
    };

    let open_bearer = token(store, open, open_admin, caps()).await;
    let res = post(open_bearer.clone(), a_thread(store, open).await).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let body: Value = res.json().await.unwrap_or(Value::Null);
    assert!(
        body.to_string().contains("free allowance for stored_bytes"),
        "{body}"
    );
    // Reading still works, so the workspace can see what to clear.
    assert_eq!(
        status(&h, &open_bearer, &format!("/workspaces/{}", open.0))
            .await
            .0,
        StatusCode::OK
    );

    for (ws, who) in [
        (other, other_admin),
        (invited, invited_admin),
        (ops, ops_admin),
    ] {
        let bearer = token(store, ws, who, caps()).await;
        let res = post(bearer, a_thread(store, ws).await).await;
        assert!(res.status().is_success(), "{}", res.status());
    }
}

/// Past its stored-bytes allowance an open workspace is refused at the
/// store's write, so every way of adding a message is refused alike: seeding
/// a thread from a message and an A2A `message:send`, not only the routes
/// that once listed it. And it cannot import its way to a fresh workspace,
/// as a new one or over itself.
#[tokio::test]
async fn past_its_stored_bytes_an_open_workspace_cannot_seed_send_over_a2a_or_import() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = std::sync::Arc::new(UsageAllowance::parse("stored_bytes=10").unwrap());
        s.attach_export_signing(maidan_auth::ExportSigningKey::from_seed([0x22; 32]));
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let (open, admin) = signed_up(store, "ada", true).await;
    let thread = a_thread(store, open).await;
    let source = store
        .post_message(NewMessage {
            thread_id: thread,
            author_id: admin,
            body: "before the cutoff".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap();
    let bearer = token(
        store,
        open,
        admin,
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::WORKSPACE_WRITE.to_string(),
            capability::MESSAGE_POST.to_string(),
            capability::TOKEN_ADMIN.to_string(),
        ],
    )
    .await;
    let bundle: Value = h
        .send(
            Method::GET,
            &format!("/workspaces/{}/export", open.0),
            &bearer,
            None,
        )
        .await
        .json()
        .await
        .unwrap();

    let month = UsageMonth::of(Utc::now().date_naive()).label();
    let mark = OverAllowance {
        workspace_id: open,
        meter: UsageMeter::StoredBytes,
        month,
        quantity: 11,
        allowance: 10,
        marked_at: Utc::now(),
    };
    store.mark_over_allowance(&mark, &[]).await.unwrap();
    maidan_server::usage_meters::refresh_cut_off(&state).await;

    let res = h
        .send(
            Method::POST,
            &format!("/messages/{}/seed", source.id.0),
            &bearer,
            Some(serde_json::json!({ "title": "branch", "inclusion": "quote" })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "seed");

    let res = h
        .client
        .post(h.url("/a2a/v1/message:send"))
        .bearer_auth(&bearer)
        .header("A2A-Version", "1.0")
        .json(&serde_json::json!({ "message": {
            "messageId": "m-1", "role": "ROLE_USER", "parts": [{ "text": "hello" }],
        } }))
        .send()
        .await
        .unwrap();
    let status = res.status();
    let body: Value = res.json().await.unwrap_or(Value::Null);
    assert!(!status.is_success(), "a2a send: {status} {body}");
    assert!(
        body.to_string().contains("free allowance for stored_bytes"),
        "a2a send: {body}"
    );

    for mode in ["new", "restore&force=true"] {
        let res = h
            .send(
                Method::POST,
                &format!("/workspaces/import?mode={mode}"),
                &bearer,
                Some(bundle.clone()),
            )
            .await;
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "import {mode}");
    }
    assert_eq!(
        store.workspace_origin(open).await.unwrap(),
        WorkspaceOrigin::Open
    );
}

/// Two open tenants, A cut off on stored bytes and B not. B's writes go on
/// (messages, uploads); A cannot write, cannot clone B's bundle into a fresh
/// workspace, and cannot restore over B. B keeps its origin and its data.
#[tokio::test]
async fn a_cut_off_tenant_cannot_import_into_or_restore_over_another_that_still_writes() {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = std::sync::Arc::new(UsageAllowance::parse("stored_bytes=10").unwrap());
        s.attach_export_signing(maidan_auth::ExportSigningKey::from_seed([0x23; 32]));
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let caps = || {
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::WORKSPACE_WRITE.to_string(),
            capability::MESSAGE_POST.to_string(),
            capability::ARTIFACT_UPLOAD.to_string(),
            capability::TOKEN_ADMIN.to_string(),
        ]
    };
    let (a, a_admin) = signed_up(store, "ada", true).await;
    let (b, b_admin) = signed_up(store, "bea", true).await;
    let a_bearer = token(store, a, a_admin, caps()).await;
    let b_bearer = token(store, b, b_admin, caps()).await;
    let a_thread_id = a_thread(store, a).await;
    let b_thread_id = a_thread(store, b).await;
    let b_bundle: Value = h
        .send(
            Method::GET,
            &format!("/workspaces/{}/export", b.0),
            &b_bearer,
            None,
        )
        .await
        .json()
        .await
        .unwrap();

    let mark = OverAllowance {
        workspace_id: a,
        meter: UsageMeter::StoredBytes,
        month: UsageMonth::of(Utc::now().date_naive()).label(),
        quantity: 11,
        allowance: 10,
        marked_at: Utc::now(),
    };
    store.mark_over_allowance(&mark, &[]).await.unwrap();
    maidan_server::usage_meters::refresh_cut_off(&state).await;
    assert!(state.usage_meters.is_cut_off(a, UsageMeter::StoredBytes));
    assert!(!state.usage_meters.is_cut_off(b, UsageMeter::StoredBytes));

    async fn post_to(h: &Harness, bearer: &str, thread: ThreadId) -> reqwest::Response {
        h.send(
            Method::POST,
            &format!("/threads/{}/messages", thread.0),
            bearer,
            Some(serde_json::json!({ "body": "hello there" })),
        )
        .await
    }
    let upload = |bearer: &str, bytes: &'static [u8]| {
        h.client
            .post(h.url("/artifacts?kind=attachment"))
            .bearer_auth(bearer)
            .body(bytes)
            .send()
    };
    assert_eq!(
        post_to(&h, &a_bearer, a_thread_id).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        upload(&a_bearer, b"a's bytes past the cutoff")
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // A cannot post into B's thread either.
    assert!(!post_to(&h, &a_bearer, b_thread_id)
        .await
        .status()
        .is_success());

    // B is untouched by A's cutoff.
    assert!(post_to(&h, &b_bearer, b_thread_id)
        .await
        .status()
        .is_success());
    assert!(upload(&b_bearer, b"b's bytes")
        .await
        .unwrap()
        .status()
        .is_success());

    // A cannot clone B's bundle into a fresh workspace.
    let before = store.count_workspaces().await.unwrap();
    let res = h
        .send(
            Method::POST,
            "/workspaces/import?mode=new",
            &a_bearer,
            Some(b_bundle.clone()),
        )
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS, "A import new");
    assert_eq!(store.count_workspaces().await.unwrap(), before);
    // Nor restore over B.
    let res = h
        .send(
            Method::POST,
            "/workspaces/import?mode=restore&force=true",
            &a_bearer,
            Some(b_bundle.clone()),
        )
        .await;
    assert!(
        !res.status().is_success(),
        "A restore over B: {}",
        res.status()
    );
    // B still exists, still open, still has its message, and still writes.
    assert_eq!(
        store.workspace_origin(b).await.unwrap(),
        WorkspaceOrigin::Open
    );
    assert_eq!(
        store.workspace_origin(a).await.unwrap(),
        WorkspaceOrigin::Open
    );
    assert!(post_to(&h, &b_bearer, b_thread_id)
        .await
        .status()
        .is_success());
    // B may restore itself (it is not cut off) and stays open.
    let res = h
        .send(
            Method::POST,
            "/workspaces/import?mode=restore&force=true",
            &b_bearer,
            Some(b_bundle),
        )
        .await;
    assert!(res.status().is_success(), "B restore: {}", res.status());
    assert_eq!(
        store.workspace_origin(b).await.unwrap(),
        WorkspaceOrigin::Open
    );
}

/// Past its stored-bytes allowance an open workspace's upload is refused
/// before the bytes are written, over REST, a thread context snapshot and
/// MCP alike. And when the cutoff lands between the put and the row, the
/// refused bytes are reaped, unless another workspace already holds them.
#[tokio::test]
async fn a_refused_upload_leaves_no_bytes_behind_over_rest_or_mcp() {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use bytes::Bytes;
    use maidan_artifacts::Sha256;

    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        s.usage_allowance = std::sync::Arc::new(UsageAllowance::parse("stored_bytes=10").unwrap());
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let (open, open_admin) = signed_up(store, "ada", true).await;
    let (other, other_admin) = signed_up(store, "bea", true).await;
    let caps = vec![
        capability::WORKSPACE_READ.to_string(),
        capability::ARTIFACT_UPLOAD.to_string(),
    ];
    let bearer = token(store, open, open_admin, caps.clone()).await;
    let other_bearer = token(store, other, other_admin, caps).await;
    let thread = a_thread(store, open).await;
    state
        .usage_meters
        .set_cut_off([(open, UsageMeter::StoredBytes)]);

    // Pre-check: REST upload, thread snapshot, MCP upload.
    let rest_bytes: &'static [u8] = b"rest bytes past the cutoff";
    let res = h
        .client
        .post(h.url("/artifacts?kind=attachment"))
        .bearer_auth(&bearer)
        .body(rest_bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(!state
        .artifacts
        .exists(&Sha256::compute(rest_bytes))
        .await
        .unwrap());

    let res = h
        .send(
            Method::POST,
            &format!("/threads/{}/context/snapshot", thread.0),
            &bearer,
            Some(serde_json::json!({})),
        )
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    let (rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM maidan_artifacts")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "nothing was stored for the refused uploads");

    let mcp_bytes = b"mcp bytes past the cutoff";
    let said: Value = h
        .client
        .post(h.url("/mcp"))
        .bearer_auth(&bearer)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "upload_artifact",
                "arguments": {"kind": "attachment", "content_base64": STANDARD.encode(mcp_bytes)}
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        said.to_string().contains("free allowance for stored_bytes"),
        "{said}"
    );
    assert!(!state
        .artifacts
        .exists(&Sha256::compute(mcp_bytes))
        .await
        .unwrap());

    // Race: the bytes were put, then the cutoff landed before the row.
    let auth = |ws, m| maidan_auth::AuthContext::from_session(m, ws, vec![]);
    let new = |sha: &Sha256, m| NewArtifact {
        sha256: sha.to_string(),
        size_bytes: 8,
        mime_type: None,
        filename: None,
        kind: ArtifactKind::Attachment,
        uploaded_by: Some(m),
    };
    let raced = state
        .artifacts
        .put(Bytes::from_static(b"raced in"))
        .await
        .unwrap();
    let refused = maidan_server::routes::upsert_uploaded_artifact(
        &state,
        &auth(open, open_admin),
        new(&raced, open_admin),
    )
    .await;
    assert!(refused.is_err());
    assert!(!state.artifacts.exists(&raced).await.unwrap());

    // Same race, but another workspace already holds these bytes: they stay.
    let shared: &'static [u8] = b"held by bea";
    let res = h
        .client
        .post(h.url("/artifacts?kind=attachment"))
        .bearer_auth(&other_bearer)
        .body(shared)
        .send()
        .await
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    let shared_sha = state
        .artifacts
        .put(Bytes::from_static(shared))
        .await
        .unwrap();
    let refused = maidan_server::routes::upsert_uploaded_artifact(
        &state,
        &auth(open, open_admin),
        new(&shared_sha, open_admin),
    )
    .await;
    assert!(refused.is_err());
    assert!(state.artifacts.exists(&shared_sha).await.unwrap());
}

#[tokio::test]
async fn a_workspace_past_its_embedding_allowance_cannot_reindex_itself_while_another_can_and_is_metered(
) {
    let mut state: Option<AppState> = None;
    let h = spawn_with(|s| {
        state = Some(s.clone());
    })
    .await;
    let state = state.unwrap();
    let store = h.store.as_ref();
    let (cut, cut_admin) = signed_up(store, "ada", true).await;
    let (open, open_admin) = signed_up(store, "bea", true).await;
    let caps = vec![
        capability::WORKSPACE_READ.to_string(),
        capability::WORKSPACE_WRITE.to_string(),
    ];
    let cut_bearer = token(store, cut, cut_admin, caps.clone()).await;
    let open_bearer = token(store, open, open_admin, caps).await;
    state
        .usage_meters
        .set_cut_off([(cut, UsageMeter::EmbeddingCalls)]);
    let _ = state.usage_meters.take();

    let res = h
        .send(
            Method::POST,
            "/operator/reindex-embeddings",
            &cut_bearer,
            Some(serde_json::json!({ "workspace_id": cut.0 })),
        )
        .await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);

    let res = h
        .send(
            Method::POST,
            "/operator/reindex-embeddings",
            &open_bearer,
            Some(serde_json::json!({ "workspace_id": open.0 })),
        )
        .await;
    assert_eq!(
        res.status(),
        StatusCode::ACCEPTED,
        "another workspace still reindexes"
    );
    let job: Value = res.json().await.unwrap_or(Value::Null);
    let job_id = job["job_id"].as_str().unwrap_or_default().to_string();
    let mut processed = None;
    for _ in 0..100 {
        let (_, body) = status(
            &h,
            &open_bearer,
            &format!("/operator/reindex-embeddings/{job_id}"),
        )
        .await;
        if body["status"] != "running" {
            processed = Some(
                body["processed"].as_u64().unwrap_or(0) + body["failed"].as_u64().unwrap_or(0),
            );
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let processed = processed.expect("the reindex job finishes");
    let counts = state.usage_meters.take();
    let open_calls: i64 = counts
        .iter()
        .filter(|r| r.workspace_id == open && r.meter == UsageMeter::EmbeddingCalls)
        .map(|r| r.quantity)
        .sum();
    assert_eq!(open_calls as u64, processed, "the reindex is metered");
    assert!(
        counts
            .iter()
            .all(|r| r.workspace_id != cut || r.meter != UsageMeter::EmbeddingCalls),
        "the refused workspace is metered no embedding calls"
    );
}
