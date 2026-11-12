//! Resource subscriptions and `notifications/resources/updated`: who
//! hears an update, per session, per workspace and across replicas.

use super::*;

fn member_auth(member_id: MemberId, workspace_id: WorkspaceId) -> AuthContext {
    AuthContext::from_session(
        member_id,
        workspace_id,
        vec![
            maidan_auth::capability::MESSAGE_POST.to_string(),
            maidan_auth::capability::WORKSPACE_READ.to_string(),
        ],
    )
}

async fn add_member(server: &McpServer, ws: WorkspaceId, handle: &str) -> MemberId {
    server
        .store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id
}

async fn subscribe_in(
    server: &McpServer,
    auth: &AuthContext,
    session: &McpSession,
    uri: &str,
) -> JsonRpcResponse {
    server
        .handle_in(
            request(1, "resources/subscribe", json!({ "uri": uri })),
            auth,
            session,
        )
        .await
}

async fn post(server: &McpServer, auth: &AuthContext, thread_id: ThreadId) {
    let resp = server
        .handle(
            request(
                2,
                "tools/call",
                json!({
                    "name": "post_message",
                    "arguments": { "thread_id": thread_id.0, "body": "hello" }
                }),
            ),
            auth,
        )
        .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
}

fn uris(notifications: &[JsonRpcNotification]) -> Vec<String> {
    notifications
        .iter()
        .map(|n| n.params["uri"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn subscribe_and_post_message_emits_resource_updated_notification() {
    let (server, thread_id, member_id) = mk_server().await;
    let member = server.store.get_member(member_id).await.unwrap();
    let auth = member_auth(member_id, member.workspace_id);
    let uri = format!("maidan://threads/{}", thread_id.0);
    let mut listener = server.listen(&auth, McpSession::Stdio).await;

    let subscribe = subscribe_in(&server, &auth, &McpSession::Stdio, &uri).await;
    assert!(subscribe.error.is_none());
    post(&server, &auth, thread_id).await;

    let notifications = listener.drain();
    assert_eq!(notifications.len(), 1);
    assert_eq!(notifications[0].method, NOTIFY_RESOURCE_UPDATED);
    assert_eq!(notifications[0].params["uri"], uri);
}

#[tokio::test]
async fn unsubscribe_suppresses_future_notifications() {
    let (server, thread_id, member_id) = mk_server().await;
    let member = server.store.get_member(member_id).await.unwrap();
    let auth = member_auth(member_id, member.workspace_id);
    let uri = format!("maidan://threads/{}", thread_id.0);
    let mut listener = server.listen(&auth, McpSession::Stdio).await;

    subscribe_in(&server, &auth, &McpSession::Stdio, &uri).await;
    let unsubscribe = server
        .handle_in(
            request(2, "resources/unsubscribe", json!({ "uri": uri })),
            &auth,
            &McpSession::Stdio,
        )
        .await;
    assert_eq!(unsubscribe.result.unwrap()["removed"], true);
    post(&server, &auth, thread_id).await;

    assert!(listener.drain().is_empty());
    assert_eq!(server.resource_subscription_count(), 0);
}

/// A tenant cannot subscribe to another tenant's resources or read its
/// workspace record, and its listener receives nothing of theirs — not
/// even while a second member of the other workspace watches the same URI.
#[tokio::test]
async fn resource_notifications_stay_in_the_subscribers_workspace() {
    let (server, thread_a, member_a) = mk_server().await;
    let ws_a = server
        .store
        .get_member(member_a)
        .await
        .unwrap()
        .workspace_id;
    let ws_b = server
        .store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();
    let member_b = add_member(&server, ws_b.id, "bob").await;
    let auth_a = member_auth(member_a, ws_a);
    let auth_b = member_auth(member_b, ws_b.id);
    let uri_a = format!("maidan://threads/{}", thread_a.0);
    let call = |id: i64, method: &str, uri: &str| request(id, method, json!({ "uri": uri }));

    let foreign = server
        .handle(call(1, "resources/subscribe", &uri_a), &auth_b)
        .await;
    assert!(foreign.error.is_some(), "B subscribed to A's thread");
    let foreign_ws = format!("maidan://workspaces/{}", ws_a.0);
    let read = server
        .handle(call(2, "resources/read", &foreign_ws), &auth_b)
        .await;
    assert!(read.error.is_some(), "B read A's workspace record");
    let own_ws = format!("maidan://workspaces/{}", ws_b.id.0);
    assert!(server
        .handle(call(3, "resources/read", &own_ws), &auth_b)
        .await
        .error
        .is_none());

    let auth_carol = member_auth(add_member(&server, ws_a, "carol").await, ws_a);
    let mut listen_a = server.listen(&auth_a, McpSession::Stateless).await;
    let mut listen_b = server.listen(&auth_b, McpSession::Stateless).await;
    let mut listen_carol = server.listen(&auth_carol, McpSession::Stateless).await;
    assert!(
        subscribe_in(&server, &auth_a, &McpSession::Stateless, &uri_a)
            .await
            .error
            .is_none()
    );
    assert!(
        subscribe_in(&server, &auth_carol, &McpSession::Stateless, &uri_a)
            .await
            .error
            .is_none()
    );
    post(&server, &auth_a, thread_a).await;

    assert!(uris(&listen_a.drain()).contains(&uri_a));
    assert!(uris(&listen_carol.drain()).contains(&uri_a));
    assert!(
        listen_b.drain().is_empty(),
        "B's listener received A's update"
    );
}

/// Subscriptions were keyed by workspace, so every listener in the
/// workspace heard what any one member subscribed to, and one member's
/// unsubscribe silenced everyone else's.
#[tokio::test]
async fn a_subscription_is_heard_only_by_the_caller_and_session_that_made_it() {
    let (server, thread_id, alice) = mk_server().await;
    let ws = server.store.get_member(alice).await.unwrap().workspace_id;
    let bob = add_member(&server, ws, "bob").await;
    let auth_alice = member_auth(alice, ws);
    let auth_bob = member_auth(bob, ws);
    let uri = format!("maidan://threads/{}", thread_id.0);
    let alice_session = McpSession::Streamable("alice-session".into());
    server
        .streamable_sessions()
        .open("alice-session".into(), Principal::of(&auth_alice))
        .await;

    let mut alice_stateless = server.listen(&auth_alice, McpSession::Stateless).await;
    let mut alice_in_session = server.listen(&auth_alice, alice_session.clone()).await;
    let mut bob_stateless = server.listen(&auth_bob, McpSession::Stateless).await;
    assert!(subscribe_in(&server, &auth_alice, &alice_session, &uri)
        .await
        .error
        .is_none());
    // Bob unsubscribing from what he never subscribed to changes nothing.
    let bob_unsub = server
        .handle(
            request(3, "resources/unsubscribe", json!({ "uri": uri })),
            &auth_bob,
        )
        .await;
    assert_eq!(bob_unsub.result.unwrap()["removed"], false);
    post(&server, &auth_bob, thread_id).await;

    assert_eq!(uris(&alice_in_session.drain()), vec![uri.clone()]);
    assert!(
        alice_stateless.drain().is_empty(),
        "a subscription made in a session leaked to another of the caller's listeners"
    );
    assert!(
        bob_stateless.drain().is_empty(),
        "another member received a subscription he never made"
    );
}

/// Access is re-checked at delivery the way the live event streams bind
/// private channels: a member outside a private channel never hears about
/// its threads, and a member removed from one stops hearing and loses the
/// subscription.
#[tokio::test]
async fn a_private_thread_update_reaches_only_members_who_can_still_read_it() {
    let (server, _public_thread, alice) = mk_server().await;
    let ws = server.store.get_member(alice).await.unwrap().workspace_id;
    let bob = add_member(&server, ws, "bob").await;
    let private = server
        .store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap();
    for member in [alice, bob] {
        server
            .store
            .add_channel_member(private.id, member, ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    let thread = server
        .store
        .create_thread(NewThread {
            channel_id: private.id,
            parent_thread_id: None,
            title: Some("secret".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let auth_alice = member_auth(alice, ws);
    let auth_bob = member_auth(bob, ws);
    let carol = add_member(&server, ws, "carol").await;
    let auth_carol = member_auth(carol, ws);
    let uri = format!("maidan://threads/{}", thread.id.0);

    let mut alice_listener = server.listen(&auth_alice, McpSession::Stateless).await;
    let mut bob_listener = server.listen(&auth_bob, McpSession::Stateless).await;
    let mut carol_listener = server.listen(&auth_carol, McpSession::Stateless).await;
    let refused = subscribe_in(&server, &auth_carol, &McpSession::Stateless, &uri).await;
    assert!(refused.error.is_some(), "a non-member subscribed");
    for auth in [&auth_alice, &auth_bob] {
        assert!(subscribe_in(&server, auth, &McpSession::Stateless, &uri)
            .await
            .error
            .is_none());
    }
    post(&server, &auth_alice, thread.id).await;
    assert_eq!(uris(&alice_listener.drain()), vec![uri.clone()]);
    assert_eq!(uris(&bob_listener.drain()), vec![uri.clone()]);
    assert!(carol_listener.drain().is_empty());

    server
        .store
        .remove_channel_member(private.id, bob)
        .await
        .unwrap();
    post(&server, &auth_alice, thread.id).await;
    assert_eq!(uris(&alice_listener.drain()), vec![uri.clone()]);
    assert!(
        bob_listener.drain().is_empty(),
        "a member removed from the channel still heard about it"
    );
    let keys: Vec<String> = [&auth_alice, &auth_bob]
        .iter()
        .map(|auth| Principal::of(auth).key())
        .collect();
    let left = server
        .store
        .mcp_resource_watchers(ws, std::slice::from_ref(&uri), &keys, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(
        left.iter().map(|w| &w.subscriber).collect::<Vec<_>>(),
        vec![&keys[0]],
        "the removed member's subscription outlived his access"
    );
}

/// Artifacts are content-addressed and shared: two workspaces that
/// uploaded the same bytes both hold `maidan://artifacts/{sha}`. One
/// workspace's upload is not the other's news.
#[tokio::test]
async fn an_artifact_update_reaches_only_the_workspace_it_happened_in() {
    let (server, _thread, member_a) = mk_server().await;
    let ws_a = server
        .store
        .get_member(member_a)
        .await
        .unwrap()
        .workspace_id;
    let ws_b = server
        .store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let member_b = add_member(&server, ws_b, "bob").await;
    let sha = "ab".repeat(32);
    server
        .store
        .upsert_artifact(NewArtifact {
            sha256: sha.clone(),
            size_bytes: 1,
            mime_type: None,
            filename: None,
            kind: ArtifactKind::Attachment,
            uploaded_by: None,
        })
        .await
        .unwrap();
    for ws in [ws_a, ws_b] {
        server.store.record_artifact_ref(ws, &sha).await.unwrap();
    }
    let uri = format!("maidan://artifacts/{sha}");
    let auth_a = member_auth(member_a, ws_a);
    let auth_b = member_auth(member_b, ws_b);
    let mut listen_a = server.listen(&auth_a, McpSession::Stateless).await;
    let mut listen_b = server.listen(&auth_b, McpSession::Stateless).await;
    // A subscribes with the uppercase digest. The stored subscription is
    // the canonical URI, which is what an upload publishes.
    let upper = format!("maidan://artifacts/{}", sha.to_ascii_uppercase());
    assert!(
        subscribe_in(&server, &auth_a, &McpSession::Stateless, &upper)
            .await
            .error
            .is_none()
    );
    assert!(subscribe_in(&server, &auth_b, &McpSession::Stateless, &uri)
        .await
        .error
        .is_none());

    server
        .publish_resource_uris(vec![ResourceUpdate::new(ws_a, uri.clone())])
        .await;
    assert_eq!(uris(&listen_a.drain()), vec![uri.clone()]);
    assert!(
        listen_b.drain().is_empty(),
        "another workspace heard about this workspace's upload"
    );
}

#[tokio::test]
async fn closing_a_session_ends_its_subscriptions() {
    let (server, thread_id, member_id) = mk_server().await;
    let ws = server
        .store
        .get_member(member_id)
        .await
        .unwrap()
        .workspace_id;
    let auth = member_auth(member_id, ws);
    let stranger = member_auth(add_member(&server, ws, "mallory").await, ws);
    server
        .streamable_sessions()
        .open("s1".into(), Principal::of(&auth))
        .await;
    let session = McpSession::Streamable("s1".into());
    let uri = format!("maidan://threads/{}", thread_id.0);
    assert!(subscribe_in(&server, &auth, &session, &uri)
        .await
        .error
        .is_none());
    assert_eq!(server.resource_subscription_count(), 1);

    server.close_streamable_session("s1", &stranger).await;
    assert_eq!(
        server.resource_subscription_count(),
        1,
        "another caller closed the session"
    );
    server.close_streamable_session("s1", &auth).await;
    assert_eq!(server.resource_subscription_count(), 0);
    assert_eq!(server.reap_streamable_sessions().await, 0);
}

#[tokio::test]
async fn resource_notifier_delivers_to_local_subscribers_via_listener() {
    use maidan_bus::InMemoryResourceNotifier;

    let (server, thread_id, member_id) = mk_server().await;
    let notifier = Arc::new(InMemoryResourceNotifier::new());
    let server = Arc::new(server.with_resource_notifier(notifier));
    server.spawn_resource_notify_listener();

    let ws = server
        .store
        .get_member(member_id)
        .await
        .unwrap()
        .workspace_id;
    let auth = admin_auth(member_id, ws);
    let uri = format!("maidan://threads/{}", thread_id.0);
    let mut listener = server.listen(&auth, McpSession::Stateless).await;
    subscribe_in(&server, &auth, &McpSession::Stateless, &uri).await;
    server
        .publish_resource_uris(vec![ResourceUpdate::new(ws, uri.clone())])
        .await;

    // Delivery routes through the notifier and the listener loop.
    let got = tokio::time::timeout(std::time::Duration::from_secs(2), listener.recv())
        .await
        .expect("timed out waiting for notification")
        .expect("notification channel closed");
    assert_eq!(got.method, NOTIFY_RESOURCE_UPDATED);
    assert_eq!(got.params["uri"], uri);
}

/// Another replica over the same store, fanned out to over `notifier`.
fn replica(
    server: &McpServer,
    notifier: &Arc<maidan_bus::InMemoryResourceNotifier>,
    idle_ttl: std::time::Duration,
) -> Arc<McpServer> {
    let replica = Arc::new(
        McpServer::new(
            server.store.clone(),
            server.artifacts.clone(),
            server.search.clone(),
            server.embedding_provider.clone(),
        )
        .with_resource_subscription_idle_ttl(idle_ttl)
        .with_resource_notifier(notifier.clone()),
    );
    replica.spawn_resource_notify_listener();
    replica
}

async fn next_uri(listener: &mut NotificationListener, within_ms: u64) -> Option<String> {
    tokio::time::timeout(std::time::Duration::from_millis(within_ms), listener.recv())
        .await
        .ok()
        .flatten()
        .map(|n| n.params["uri"].as_str().unwrap_or_default().to_string())
}

/// A stateless caller has no session to pin it to a replica, so behind a
/// load balancer its subscribe and its listener land wherever they land.
/// The subscription was held by the replica that took it, and a listener
/// on any other replica heard nothing.
#[tokio::test]
async fn a_stateless_subscription_taken_on_one_replica_is_delivered_by_another() {
    let (server, thread_id, member_id) = mk_server().await;
    let ws = server
        .store
        .get_member(member_id)
        .await
        .unwrap()
        .workspace_id;
    let notifier = Arc::new(maidan_bus::InMemoryResourceNotifier::new());
    let hour = std::time::Duration::from_secs(3600);
    let a = replica(&server, &notifier, hour);
    let b = replica(&server, &notifier, hour);
    let alice = member_auth(member_id, ws);
    let uri = format!("maidan://threads/{}", thread_id.0);

    let mut on_b = b.listen(&alice, McpSession::Stateless).await;
    let mut on_a = a.listen(&alice, McpSession::Stateless).await;
    assert!(subscribe_in(&a, &alice, &McpSession::Stateless, &uri)
        .await
        .error
        .is_none());
    post(&a, &alice, thread_id).await;
    assert_eq!(next_uri(&mut on_b, 2000).await, Some(uri.clone()));
    assert_eq!(
        next_uri(&mut on_a, 2000).await,
        Some(uri.clone()),
        "the replica that took it delivers it too"
    );
    assert_eq!(next_uri(&mut on_b, 300).await, None, "delivered twice");

    // Unsubscribing on B ends the subscription A took.
    let unsub = b
        .handle(
            request(3, "resources/unsubscribe", json!({ "uri": uri })),
            &alice,
        )
        .await;
    assert_eq!(unsub.result.unwrap()["removed"], true);
    post(&a, &alice, thread_id).await;
    assert_eq!(next_uri(&mut on_b, 300).await, None);
    assert_eq!(next_uri(&mut on_a, 100).await, None);
}

/// Across replicas an update still reaches only the workspace it
/// happened in: two tenants watching the same content-addressed artifact,
/// each subscribed on one replica and listening on the other.
#[tokio::test]
async fn a_stateless_subscription_on_another_replica_hears_only_its_own_workspace() {
    let (server, _thread, member_a) = mk_server().await;
    let ws_a = server
        .store
        .get_member(member_a)
        .await
        .unwrap()
        .workspace_id;
    let ws_b = server
        .store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap()
        .id;
    let member_b = add_member(&server, ws_b, "bob").await;
    let sha = "c".repeat(64);
    server
        .store
        .upsert_artifact(NewArtifact {
            sha256: sha.clone(),
            size_bytes: 1,
            mime_type: None,
            kind: ArtifactKind::Attachment,
            uploaded_by: None,
            filename: None,
        })
        .await
        .unwrap();
    for ws in [ws_a, ws_b] {
        server.store.record_artifact_ref(ws, &sha).await.unwrap();
    }
    let uri = format!("maidan://artifacts/{sha}");
    let notifier = Arc::new(maidan_bus::InMemoryResourceNotifier::new());
    let hour = std::time::Duration::from_secs(3600);
    let one = replica(&server, &notifier, hour);
    let two = replica(&server, &notifier, hour);
    let auth_a = member_auth(member_a, ws_a);
    let auth_b = member_auth(member_b, ws_b);

    let mut a_on_two = two.listen(&auth_a, McpSession::Stateless).await;
    let mut b_on_one = one.listen(&auth_b, McpSession::Stateless).await;
    let mut b_on_two = two.listen(&auth_b, McpSession::Stateless).await;
    for (on, auth) in [(&one, &auth_a), (&two, &auth_b)] {
        assert!(subscribe_in(on, auth, &McpSession::Stateless, &uri)
            .await
            .error
            .is_none());
    }

    one.publish_resource_uris(vec![ResourceUpdate::new(ws_a, uri.clone())])
        .await;
    assert_eq!(next_uri(&mut a_on_two, 2000).await, Some(uri.clone()));
    assert_eq!(
        next_uri(&mut b_on_one, 300).await,
        None,
        "another workspace heard about this workspace's upload"
    );
    assert_eq!(next_uri(&mut b_on_two, 100).await, None);

    two.publish_resource_uris(vec![ResourceUpdate::new(ws_b, uri.clone())])
        .await;
    assert_eq!(next_uri(&mut b_on_one, 2000).await, Some(uri.clone()));
    assert_eq!(next_uri(&mut b_on_two, 2000).await, Some(uri.clone()));
    assert_eq!(next_uri(&mut a_on_two, 300).await, None);
}

/// A listener on any replica keeps a stateless subscription alive past
/// the idle TTL. Once no replica listens (the listener closed, or its
/// replica died and stopped extending it), it lapses and is reaped, and a
/// later listener does not bring it back.
#[tokio::test]
async fn a_stateless_subscription_lives_while_any_replica_listens_then_lapses() {
    let (server, thread_id, member_id) = mk_server().await;
    let ws = server
        .store
        .get_member(member_id)
        .await
        .unwrap()
        .workspace_id;
    let notifier = Arc::new(maidan_bus::InMemoryResourceNotifier::new());
    // Long enough that upkeep runs its four passes per TTL (300 ms each)
    // rather than three at its 100 ms floor, so a loaded runner that
    // delays one pass does not lapse a subscription a listener holds.
    let ttl = std::time::Duration::from_millis(1200);
    let a = replica(&server, &notifier, ttl);
    let b = replica(&server, &notifier, ttl);
    let alice = member_auth(member_id, ws);
    let uri = format!("maidan://threads/{}", thread_id.0);

    let mut on_b = b.listen(&alice, McpSession::Stateless).await;
    assert!(subscribe_in(&a, &alice, &McpSession::Stateless, &uri)
        .await
        .error
        .is_none());
    tokio::time::sleep(ttl * 4).await;
    post(&a, &alice, thread_id).await;
    assert_eq!(
        next_uri(&mut on_b, 2000).await,
        Some(uri.clone()),
        "B's listener did not keep the subscription alive"
    );

    drop(on_b);
    drop(b);
    tokio::time::sleep(ttl * 4).await;
    let key = Principal::of(&alice).key();
    let far = chrono::Utc::now() - chrono::Duration::hours(1);
    assert!(
        server
            .store
            .mcp_resource_watchers(ws, std::slice::from_ref(&uri), &[key], far)
            .await
            .unwrap()
            .is_empty(),
        "the lapsed subscription was not reaped"
    );
    let mut on_a = a.listen(&alice, McpSession::Stateless).await;
    post(&a, &alice, thread_id).await;
    assert_eq!(next_uri(&mut on_a, 300).await, None);
}
