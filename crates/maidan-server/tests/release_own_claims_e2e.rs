//! `POST /workspaces/{id}/claims/release-own`, the REST twin of MCP
//! `release_own_claims`: a claimant that restarted frees every live claim its
//! own member holds in its own workspace, without the fencing tokens the old
//! process took with it. The member is the token's, so another member's
//! claims are never touched; another workspace is refused, never read as
//! "all of them"; and the status the holder declared goes with each claim.

mod seeded_workspace;

use maidan_auth::capability;
use maidan_store::prelude::*;
use maidan_types::{
    ChannelId, DeclaredStatus, EventKind, MemberId, MemberKind, NewChannel, NewThread,
    NewWorkspace, Thread, ThreadId, WorkspaceId, MAX_RELEASE_NOTE_CHARS,
};
use reqwest::{Method, StatusCode};
use seeded_workspace::{member, spawn, token, Harness};
use serde_json::{json, Value};

const LEASE_SECS: i64 = 600;

async fn workspace(store: &dyn Store, name: &str) -> (WorkspaceId, ChannelId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "queue".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id;
    (ws, channel)
}

/// A new task in `channel`, claimed at once by `who` on a live lease.
async fn claimed(store: &dyn Store, channel: ChannelId, who: MemberId, title: &str) -> Thread {
    let id = store
        .create_thread(NewThread {
            channel_id: channel,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id;
    let claim = store
        .claim_next_thread(channel, who, Some(LEASE_SECS), None)
        .await
        .unwrap()
        .expect("the new task is claimable");
    assert_eq!(claim.id, id);
    claim
}

async fn worker_token(store: &dyn Store, ws: WorkspaceId, who: MemberId) -> String {
    token(
        store,
        ws,
        who,
        vec![
            capability::WORKSPACE_READ.into(),
            capability::THREAD_TRANSITION.into(),
        ],
    )
    .await
}

async fn release_own(
    h: &Harness,
    bearer: &str,
    ws: WorkspaceId,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let res = h
        .send(
            Method::POST,
            &format!("/workspaces/{}/claims/release-own", ws.0),
            bearer,
            body,
        )
        .await;
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

fn ids(body: &Value) -> Vec<String> {
    body["released"]
        .as_array()
        .unwrap_or_else(|| panic!("no released list: {body}"))
        .iter()
        .map(|id| id.as_str().unwrap_or_default().to_string())
        .collect()
}

async fn holder(store: &dyn Store, id: ThreadId) -> Option<MemberId> {
    store.get_thread(id).await.unwrap().assignee_id
}

#[tokio::test]
async fn a_restarted_claimant_releases_only_its_own_claims_in_its_own_workspace() {
    let h = spawn().await;
    let store = h.store.as_ref();
    let (ws, channel) = workspace(store, "claims").await;
    let (other_ws, other_channel) = workspace(store, "other-tenant").await;
    let a = member(store, ws, "a", MemberKind::Agent).await;
    let b = member(store, ws, "b", MemberKind::Agent).await;
    let stranger = member(store, other_ws, "stranger", MemberKind::Agent).await;

    let first = claimed(store, channel, a, "a's first").await;
    let second = claimed(store, channel, a, "a's second").await;
    let bs = claimed(store, channel, b, "b's").await;
    let theirs = claimed(store, other_channel, stranger, "the stranger's").await;
    store
        .declare_thread_status(first.id, DeclaredStatus::Working, "halfway".into(), a)
        .await
        .unwrap();
    let bearer = worker_token(store, ws, a).await;

    // Without `thread:transition` nothing is released.
    let reader = token(store, ws, a, vec![capability::WORKSPACE_READ.into()]).await;
    let (status, body) = release_own(&h, &reader, ws, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(holder(store, first.id).await, Some(a));

    // Another tenant's workspace is refused, and its claim is untouched.
    let (status, body) = release_own(&h, &bearer, other_ws, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(holder(store, theirs.id).await, Some(stranger));

    // A note over the limit is refused before anything is released.
    let long = "a".repeat(MAX_RELEASE_NOTE_CHARS + 1);
    let (status, body) = release_own(&h, &bearer, ws, Some(json!({ "note": long }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(holder(store, first.id).await, Some(a));

    // No body at all: the claimant's two live claims go back to the queue.
    let (status, body) = release_own(&h, &bearer, ws, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut expected = vec![first.id, second.id];
    expected.sort_by_key(|id| id.0);
    assert_eq!(
        ids(&body),
        expected
            .iter()
            .map(|id| id.0.to_string())
            .collect::<Vec<_>>()
    );
    for id in [first.id, second.id] {
        assert_eq!(holder(store, id).await, None);
    }
    assert_eq!(
        store.get_thread_status(first.id).await.unwrap(),
        None,
        "the holder's status went with its claim"
    );
    assert_eq!(holder(store, bs.id).await, Some(b), "b's claim is b's");
    assert_eq!(holder(store, theirs.id).await, Some(stranger));

    // The old process is fenced out of what it held.
    let res = h
        .send(
            Method::POST,
            &format!("/threads/{}/claim/renew", first.id.0),
            &bearer,
            Some(json!({ "claim_lease_id": first.claim_lease_id, "lease_secs": 60 })),
        )
        .await;
    assert!(!res.status().is_success(), "{}", res.status());

    // A second call has nothing left to release; the note it gives is the
    // one each release's event carries.
    let (status, body) = release_own(&h, &bearer, ws, Some(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, json!({ "released": [] }));
    // In a channel of its own: the two released tasks are older, and the
    // queue would hand one of them out first.
    let later = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "later".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap()
        .id;
    let third = claimed(store, later, a, "a's third").await;
    let (status, body) = release_own(&h, &bearer, ws, Some(json!({ "note": "redeployed" }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body), vec![third.id.0.to_string()]);
    let events = store.list_events_after(ws, 0, 1000).await.unwrap();
    let release = events
        .iter()
        .rev()
        .find(|e| e.kind == EventKind::ThreadAssignmentChanged && e.thread_id == Some(third.id))
        .expect("the release is in the log");
    assert!(
        release.payload.to_string().contains("redeployed"),
        "the event carries the note: {}",
        release.payload
    );
}
