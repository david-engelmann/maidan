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
