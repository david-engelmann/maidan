//! The access rules in `access.rs`, asked of this crate directly. The server's
//! end-to-end suites exercise them through every route, but the nightly
//! mutation run tests each crate with its own tests only, and there every
//! denial in `access.rs` survived: replacing `can_access_thread` with
//! `Ok(true)` failed nothing here. Each rule below is asked once where it
//! allows and once where it refuses, with a second workspace for isolation.

use maidan_auth::{
    authorize_thread, can_access_channel, can_access_thread, can_read_gate, ensure_channel_access,
    ensure_dm_participant, ensure_message_access, ensure_thread_access, AuthContext, AuthError,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGate, ApprovalRisk, ChannelId, ChannelMemberRole, MemberId, MemberKind,
    NewApprovalGate, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace, ThreadId,
    WorkspaceId,
};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    SqliteStore::for_tests(pool)
}

async fn member(store: &dyn Store, workspace_id: WorkspaceId, handle: &str) -> MemberId {
    store
        .create_member(NewMember {
            workspace_id,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id
}

async fn channel(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    name: &str,
    private: bool,
) -> ChannelId {
    store
        .create_channel(NewChannel {
            workspace_id,
            name: name.into(),
            topic: None,
            private,
        })
        .await
        .unwrap()
        .id
}

async fn thread(store: &dyn Store, channel_id: ChannelId, title: &str) -> ThreadId {
    store
        .create_thread(NewThread {
            channel_id,
            parent_thread_id: None,
            title: Some(title.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id
}

async fn gate(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_id: Option<ThreadId>,
    requested_by: MemberId,
) -> ApprovalGate {
    store
        .create_approval_gate(&NewApprovalGate {
            workspace_id,
            thread_id,
            requested_by,
            prompt: "ship it?".into(),
            schema: None,
            risk: ApprovalRisk::default(),
        })
        .await
        .unwrap()
}

fn caller(member_id: MemberId, workspace_id: WorkspaceId) -> AuthContext {
    AuthContext::from_session(member_id, workspace_id, Vec::new())
}

fn forbidden<T: std::fmt::Debug>(result: Result<T, AuthError>, what: &str) {
    assert!(
        matches!(result, Err(AuthError::Forbidden(_))),
        "{what}: {result:?}"
    );
}

#[tokio::test]
async fn each_access_rule_allows_its_reader_and_refuses_everyone_else() {
    let store = sqlite().await;
    let store: &dyn Store = &store;
    let a = store
        .create_workspace(NewWorkspace {
            name: "rules-a".into(),
        })
        .await
        .unwrap()
        .id;
    let b = store
        .create_workspace(NewWorkspace {
            name: "rules-b".into(),
        })
        .await
        .unwrap()
        .id;
    let alice = member(store, a, "alice").await;
    let bob = member(store, a, "bob").await;
    let carl = member(store, a, "carl").await;
    let dave = member(store, a, "dave").await;
    let zed = member(store, b, "zed").await;
    let (alice_a, bob_a, carl_a, dave_a) = (
        caller(alice, a),
        caller(bob, a),
        caller(carl, a),
        caller(dave, a),
    );
    // zed's own workspace is B; asking about A's records is the other tenant.
    let zed_b = caller(zed, b);

    let general = channel(store, a, "general", false).await;
    let secret = channel(store, a, "secret", true).await;
    store
        .add_channel_member(secret, alice, ChannelMemberRole::Member)
        .await
        .unwrap();
    let in_general = thread(store, general, "open").await;
    let in_secret = thread(store, secret, "closed").await;
    let dm = store.open_dm_conversation(a, alice, bob).await.unwrap();
    let dm_channel = store.get_thread(dm.thread_id).await.unwrap().channel_id;
    let group = store
        .open_group_dm_conversation(a, &[alice, bob, carl], None)
        .await
        .unwrap();
    let secret_message = store
        .post_message(NewMessage {
            thread_id: in_secret,
            author_id: alice,
            body: "inside".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .unwrap()
        .id;

    // A public channel is open to the workspace, a private one to its
    // members, the DM system channel to everyone (its threads are gated per
    // conversation), and nothing to another workspace.
    ensure_channel_access(store, &bob_a, general).await.unwrap();
    ensure_channel_access(store, &alice_a, secret)
        .await
        .unwrap();
    forbidden(
        ensure_channel_access(store, &bob_a, secret).await,
        "a non-member opened a private channel",
    );
    ensure_channel_access(store, &bob_a, dm_channel)
        .await
        .unwrap();
    assert!(ensure_channel_access(store, &zed_b, general).await.is_err());
    assert!(can_access_channel(store, &alice_a, secret).await.unwrap());
    assert!(!can_access_channel(store, &bob_a, secret).await.unwrap());

    // A DM reaches its two members, a group DM its members, nobody else.
    ensure_dm_participant(store, &alice_a, dm.thread_id)
        .await
        .unwrap();
    ensure_dm_participant(store, &bob_a, dm.thread_id)
        .await
        .unwrap();
    forbidden(
        ensure_dm_participant(store, &carl_a, dm.thread_id).await,
        "a third member read a DM",
    );
    ensure_dm_participant(store, &carl_a, group.thread_id)
        .await
        .unwrap();
    forbidden(
        ensure_dm_participant(store, &dave_a, group.thread_id).await,
        "a non-member read a group DM",
    );

    // A thread is as open as its channel or its conversation.
    let scope = authorize_thread(store, &bob_a, in_general).await.unwrap();
    assert_eq!(
        (scope.workspace_id, scope.channel_id, scope.thread_id),
        (a, general, in_general)
    );
    authorize_thread(store, &alice_a, in_secret).await.unwrap();
    forbidden(
        authorize_thread(store, &bob_a, in_secret).await,
        "a non-member opened a private channel's thread",
    );
    forbidden(
        ensure_thread_access(store, &bob_a, in_secret).await,
        "ensure_thread_access let a non-member in",
    );
    forbidden(
        ensure_thread_access(store, &carl_a, dm.thread_id).await,
        "a third member opened a DM thread",
    );
    assert!(ensure_thread_access(store, &zed_b, in_general)
        .await
        .is_err());
    assert!(can_access_thread(store, &alice_a, in_secret).await.unwrap());
    assert!(!can_access_thread(store, &bob_a, in_secret).await.unwrap());

    // A message is as open as its thread.
    ensure_message_access(store, &alice_a, secret_message)
        .await
        .unwrap();
    forbidden(
        ensure_message_access(store, &bob_a, secret_message).await,
        "a non-member read a private channel's message",
    );

    // A gate is no wider than its thread, and never crosses workspaces.
    let on_secret = gate(store, a, Some(in_secret), alice).await;
    let on_nothing = gate(store, a, None, alice).await;
    let elsewhere = gate(store, b, None, zed).await;
    assert!(can_read_gate(store, &alice_a, &on_secret).await.unwrap());
    assert!(!can_read_gate(store, &bob_a, &on_secret).await.unwrap());
    assert!(can_read_gate(store, &bob_a, &on_nothing).await.unwrap());
    assert!(!can_read_gate(store, &bob_a, &elsewhere).await.unwrap());
    assert!(can_read_gate(store, &zed_b, &elsewhere).await.unwrap());
}
