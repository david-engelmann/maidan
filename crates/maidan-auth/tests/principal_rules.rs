//! Who a request is, asked of this crate directly: a browser session, the
//! anonymous reader of a synthetic workspace, a member the server acts for,
//! and a bearer of a suspended workspace. Each survived the nightly's
//! per-crate mutation run: `ensure_workspace_active` could answer `Ok(())`,
//! and an expired session could pass, without a test here failing.

use chrono::{Duration, Utc};
use maidan_auth::{
    ensure_workspace_active, hash_secret, member_can_access_channel, member_can_access_thread,
    resolve_anonymous_reader, resolve_bearer, resolve_session, resolve_token_id, AuthError,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, ChannelMemberRole, MemberKind, NewApiToken, NewAuditEvent, NewChannel,
    NewMaidanSession, NewMember, NewThread, NewWorkspace, WorkspaceId, WORKSPACE_SUSPENDED,
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

async fn workspace(store: &dyn Store, name: &str) -> WorkspaceId {
    store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn a_session_lives_until_it_expires() {
    let store = sqlite().await;
    let ws = workspace(&store, "sessions").await;
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "ana".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap()
        .id;
    let session = |expires_at| NewMaidanSession {
        workspace_id: ws,
        member_id: member,
        api_token_id: None,
        oidc_identity_id: None,
        expires_at,
    };
    let live = store
        .create_session(session(Utc::now() + Duration::hours(1)))
        .await
        .unwrap();
    let principal = resolve_session(&store, live.id).await.unwrap();
    let ctx = principal.context(Vec::new());
    assert_eq!(ctx.member_id(), member);
    assert!(
        !ctx.is_anonymous(),
        "a person's session is not the anonymous reader"
    );

    let expired = store
        .create_session(session(Utc::now() - Duration::seconds(1)))
        .await
        .unwrap();
    assert!(matches!(
        resolve_session(&store, expired.id).await,
        Err(AuthError::Unauthorized)
    ));
}

#[tokio::test]
async fn only_a_synthetic_workspace_has_an_anonymous_reader() {
    let store = sqlite().await;
    let synthetic = workspace(&store, "synthetic-demo").await;
    let real = workspace(&store, "acme").await;
    let reader = resolve_anonymous_reader(&store, synthetic).await.unwrap();
    assert!(reader.is_anonymous());
    assert_eq!(reader.workspace_id(), synthetic);
    assert!(matches!(
        resolve_anonymous_reader(&store, real).await,
        Err(AuthError::Forbidden(_))
    ));
}

#[tokio::test]
async fn a_member_the_server_acts_for_reads_only_what_they_could_open() {
    let store = sqlite().await;
    let ws = workspace(&store, "recipients").await;
    let member = |handle: &str| NewMember {
        workspace_id: ws,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Human,
    };
    let inside = store.create_member(member("inside")).await.unwrap().id;
    let outside = store.create_member(member("outside")).await.unwrap().id;
    let private = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "private".into(),
            topic: None,
            private: true,
        })
        .await
        .unwrap()
        .id;
    store
        .add_channel_member(private, inside, ChannelMemberRole::Member)
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: private,
            parent_thread_id: None,
            title: Some("held".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap()
        .id;
    assert!(member_can_access_channel(&store, ws, inside, private)
        .await
        .unwrap());
    assert!(!member_can_access_channel(&store, ws, outside, private)
        .await
        .unwrap());
    assert!(member_can_access_thread(&store, ws, inside, thread)
        .await
        .unwrap());
    assert!(!member_can_access_thread(&store, ws, outside, thread)
        .await
        .unwrap());
}

#[tokio::test]
async fn a_good_bearer_of_a_suspended_workspace_is_forbidden_not_unknown() {
    let store = sqlite().await;
    let ws = workspace(&store, "suspend-me").await;
    let agent = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap()
        .id;
    // A bearer at the length limit is still a bearer.
    let secret = "m".repeat(512);
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: agent,
            app_installation_id: None,
            token_hash: hash_secret(&secret),
            label: Some("suspension".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    assert_eq!(
        resolve_bearer(&store, &secret).await.unwrap().member_id(),
        agent
    );
    // One character past the limit is refused before any lookup, even for a
    // bearer that was minted; an ordinary short bearer resolves.
    for (other, resolves) in [("m".repeat(513), false), ("short-bearer".to_string(), true)] {
        store
            .create_api_token(NewApiToken {
                workspace_id: ws,
                member_id: agent,
                app_installation_id: None,
                token_hash: hash_secret(&other),
                label: None,
                capabilities: vec!["workspace:read".into()],
                expires_at: None,
            })
            .await
            .unwrap();
        let resolved = resolve_bearer(&store, &other).await;
        assert_eq!(
            resolved.is_ok(),
            resolves,
            "{} chars: {resolved:?}",
            other.len()
        );
    }
    assert_eq!(
        resolve_token_id(&store, token.id)
            .await
            .unwrap()
            .member_id(),
        agent
    );
    ensure_workspace_active(&store, ws).await.unwrap();

    store
        .suspend_workspace(
            ws,
            "abuse",
            None,
            NewAuditEvent {
                scope: AuditScope::Workspace(ws),
                actor_id: None,
                action: "workspace.suspend".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(ws.0),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
    for refused in [
        ensure_workspace_active(&store, ws).await.map(|_| ()),
        resolve_bearer(&store, &secret).await.map(|_| ()),
        resolve_token_id(&store, token.id).await.map(|_| ()),
    ] {
        assert!(
            matches!(&refused, Err(AuthError::Forbidden(reason)) if reason == WORKSPACE_SUSPENDED),
            "{refused:?}"
        );
    }
}
