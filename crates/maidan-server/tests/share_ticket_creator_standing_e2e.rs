//! A share ticket is its creator's authority lent to a reader outside the
//! workspace, so it works only while that creator could still mint it: a live,
//! active member who can read the channel, and, when the ticket was minted
//! with an API token, a token that is still live. Each use asks again. A
//! ticket whose creator lost any of that answers as an unknown ticket does,
//! says nothing of why, and is revoked so it stays dead.
//!
//! Every case mints through the REST route with a real bearer, as an operator
//! does, so the token the ticket records is a real one.

use std::sync::Arc;

use chrono::{Duration as ChronoDuration, Utc};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, ShareTicketSecret};
use maidan_bus::InMemoryBus;
use maidan_server::test_support::{self, Caller, Tenant};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    AuditScope, ChannelId, ChannelMemberRole, MemberId, MemberKind, NewAuditEvent, NewChannel,
    NewMember, NewMessage, NewShareTicket, NewThread, ShareTicketId, ThreadId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

struct Rig {
    base: String,
    store: Arc<dyn Store>,
    tenant: Tenant,
    reader: reqwest::Client,
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
    let tenant = test_support::tenant(store.as_ref(), "standing-ws").await;
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
        reader: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap(),
    }
}

fn audit(ws: maidan_types::WorkspaceId, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(ws),
        actor_id: None,
        action: action.into(),
        target_kind: None,
        target_id: None,
        metadata: json!({}),
    }
}

impl Rig {
    /// A member the identity provider can deactivate, holding every capability.
    async fn scim_member(&self, handle: &str) -> Caller {
        let ws = self.tenant.workspace_id;
        let (member, _) = self
            .store
            .scim_provision_audited(
                NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Human,
                },
                Some(handle),
                true,
                Box::new(move |_| audit(ws, "scim.user.create")),
            )
            .await
            .unwrap();
        let token =
            test_support::token(self.store.as_ref(), ws, member.id, capability::all()).await;
        Caller {
            member_id: member.id,
            token,
        }
    }

    async fn deactivate(&self, who: MemberId, handle: &str) {
        let ws = self.tenant.workspace_id;
        self.store
            .scim_update_user_audited(
                ws,
                who,
                None,
                Some(handle),
                false,
                audit(ws, "scim.user.deactivate"),
            )
            .await
            .unwrap()
            .unwrap();
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

    async fn thread_with_message(&self, channel_id: ChannelId, author: MemberId) -> ThreadId {
        let thread = self
            .store
            .create_thread(NewThread {
                channel_id,
                parent_thread_id: None,
                title: Some("shared".into()),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
        self.store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: author,
                body: "visible to the ticket holder".into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
        thread.id
    }

    /// Mint a ticket for `channel_id` as `creator` over REST: its id and the
    /// `Authorization` value its holder sends.
    async fn mint(&self, creator: &Caller, channel_id: ChannelId) -> (ShareTicketId, String) {
        let response = creator
            .client()
            .post(format!(
                "{}/workspaces/{}/share-tickets",
                self.base, self.tenant.workspace_id.0
            ))
            .json(&json!({
                "channel_id": channel_id.0,
                "expires_at": Utc::now() + ChronoDuration::hours(1),
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let created: Value = response.json().await.unwrap();
        (
            ShareTicketId(created["ticket"]["id"].as_str().unwrap().parse().unwrap()),
            format!("ShareTicket {}", created["secret"].as_str().unwrap()),
        )
    }

    /// The status and body of one read through the ticket.
    async fn read(&self, path: &str, authorization: &str) -> (StatusCode, String) {
        let response = self
            .reader
            .get(format!("{}{path}", self.base))
            .header("authorization", authorization)
            .send()
            .await
            .unwrap();
        (response.status(), response.text().await.unwrap())
    }

    /// Every route the holder has, each answering `StatusCode::OK`.
    async fn assert_reads(&self, authorization: &str, thread: ThreadId) {
        for path in [
            "/share/manifest".to_string(),
            "/share/threads".to_string(),
            format!("/share/threads/{}/messages", thread.0),
        ] {
            let (status, body) = self.read(&path, authorization).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body}");
        }
    }

    /// Every route the holder has, each answering exactly as it does for a
    /// ticket nobody minted.
    async fn assert_refused_as_unknown(&self, authorization: &str, thread: ThreadId) {
        let unknown = format!("ShareTicket maid_share_{}", "e".repeat(64));
        for path in [
            "/share/manifest".to_string(),
            "/share/threads".to_string(),
            format!("/share/threads/{}/messages", thread.0),
        ] {
            let refused = self.read(&path, authorization).await;
            assert_eq!(refused.0, StatusCode::UNAUTHORIZED, "{path}: {}", refused.1);
            assert_eq!(
                refused,
                self.read(&path, &unknown).await,
                "{path}: a ticket whose creator lost access must answer as an unknown one"
            );
        }
    }

    async fn revoked(&self, ticket: ShareTicketId) -> bool {
        self.store
            .get_share_ticket(ticket)
            .await
            .unwrap()
            .revoked_at
            .is_some()
    }

    async fn token_id(&self, caller: &Caller) -> maidan_types::ApiTokenId {
        self.store
            .get_active_api_token_by_hash(&hash_secret(&caller.token))
            .await
            .unwrap()
            .id
    }
}

#[tokio::test]
async fn a_ticket_keeps_working_while_its_creator_keeps_access() {
    let rig = rig().await;
    let creator = rig.scim_member("creator").await;
    let other = rig.scim_member("other").await;
    let outsider = rig
        .tenant
        .member(rig.store.as_ref(), "outsider", MemberKind::Human)
        .await;
    let private = rig.channel("incident", true).await;
    for member in [creator.member_id, other.member_id] {
        rig.store
            .add_channel_member(private, member, ChannelMemberRole::Member)
            .await
            .unwrap();
    }
    let thread = rig.thread_with_message(private, creator.member_id).await;

    // The outsider is in the workspace but not the channel: minting is refused.
    let refused = outsider
        .client()
        .post(format!(
            "{}/workspaces/{}/share-tickets",
            rig.base, rig.tenant.workspace_id.0
        ))
        .json(&json!({
            "channel_id": private.0,
            "expires_at": Utc::now() + ChronoDuration::hours(1),
        }))
        .send()
        .await
        .unwrap();
    assert!(!refused.status().is_success(), "{}", refused.status());

    let (ticket, authorization) = rig.mint(&creator, private).await;
    rig.assert_reads(&authorization, thread).await;

    // Another member leaving the channel, and then the workspace, does not
    // touch a ticket that is not theirs.
    rig.store
        .remove_channel_member(private, other.member_id)
        .await
        .unwrap();
    rig.deactivate(other.member_id, "other").await;
    rig.assert_reads(&authorization, thread).await;
    assert!(!rig.revoked(ticket).await);
}

#[tokio::test]
async fn a_ticket_for_a_private_channel_stops_when_its_creator_leaves_the_channel() {
    let rig = rig().await;
    let creator = rig.scim_member("creator").await;
    let private = rig.channel("incident", true).await;
    rig.store
        .add_channel_member(private, creator.member_id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let thread = rig.thread_with_message(private, creator.member_id).await;
    let (ticket, authorization) = rig.mint(&creator, private).await;
    rig.assert_reads(&authorization, thread).await;

    rig.store
        .remove_channel_member(private, creator.member_id)
        .await
        .unwrap();
    rig.assert_refused_as_unknown(&authorization, thread).await;
    assert!(rig.revoked(ticket).await, "the ticket must stay dead");

    // Joining the channel again does not bring the ticket back.
    rig.store
        .add_channel_member(private, creator.member_id, ChannelMemberRole::Member)
        .await
        .unwrap();
    rig.assert_refused_as_unknown(&authorization, thread).await;
}

#[tokio::test]
async fn a_ticket_stops_when_its_creator_is_deactivated() {
    let rig = rig().await;
    let creator = rig.scim_member("creator").await;
    let private = rig.channel("incident", true).await;
    rig.store
        .add_channel_member(private, creator.member_id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let thread = rig.thread_with_message(private, creator.member_id).await;
    let (ticket, authorization) = rig.mint(&creator, private).await;
    rig.assert_reads(&authorization, thread).await;

    rig.deactivate(creator.member_id, "creator").await;
    rig.assert_refused_as_unknown(&authorization, thread).await;
    assert!(rig.revoked(ticket).await);
}

#[tokio::test]
async fn a_ticket_stops_when_the_token_that_minted_it_is_revoked() {
    let rig = rig().await;
    let creator = rig.scim_member("creator").await;
    let private = rig.channel("incident", true).await;
    rig.store
        .add_channel_member(private, creator.member_id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let thread = rig.thread_with_message(private, creator.member_id).await;

    // Two tokens of one member, one ticket each: revoking a token ends the
    // tickets it minted and none of the member's others.
    let second = Caller {
        member_id: creator.member_id,
        token: test_support::token(
            rig.store.as_ref(),
            rig.tenant.workspace_id,
            creator.member_id,
            capability::all(),
        )
        .await,
    };
    let (ticket, authorization) = rig.mint(&creator, private).await;
    let (kept_ticket, kept_authorization) = rig.mint(&second, private).await;
    rig.assert_reads(&authorization, thread).await;
    rig.assert_reads(&kept_authorization, thread).await;

    let token = rig.token_id(&creator).await;
    rig.store.revoke_api_token(token).await.unwrap();

    rig.assert_refused_as_unknown(&authorization, thread).await;
    assert!(rig.revoked(ticket).await);
    rig.assert_reads(&kept_authorization, thread).await;
    assert!(!rig.revoked(kept_ticket).await);
}

#[tokio::test]
async fn a_ticket_for_a_public_channel_survives_its_creator_leaving_the_channel_but_not_deactivation(
) {
    let rig = rig().await;
    let creator = rig.scim_member("creator").await;
    let public = rig.channel("announcements", false).await;
    rig.store
        .add_channel_member(public, creator.member_id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let thread = rig.thread_with_message(public, creator.member_id).await;
    let (ticket, authorization) = rig.mint(&creator, public).await;
    rig.assert_reads(&authorization, thread).await;

    // A public channel is readable by every member, so leaving it takes
    // nothing from the creator.
    rig.store
        .remove_channel_member(public, creator.member_id)
        .await
        .unwrap();
    rig.assert_reads(&authorization, thread).await;
    assert!(!rig.revoked(ticket).await);

    rig.deactivate(creator.member_id, "creator").await;
    rig.assert_refused_as_unknown(&authorization, thread).await;
    assert!(rig.revoked(ticket).await);
}

/// A ticket minted on a browser session, or before the token was recorded,
/// has no token to resolve; it is still held to its creator's standing.
#[tokio::test]
async fn a_ticket_with_no_recorded_token_is_held_to_its_creators_standing() {
    let rig = rig().await;
    let creator = rig.scim_member("creator").await;
    let private = rig.channel("incident", true).await;
    rig.store
        .add_channel_member(private, creator.member_id, ChannelMemberRole::Member)
        .await
        .unwrap();
    let thread = rig.thread_with_message(private, creator.member_id).await;
    let secret = ShareTicketSecret::generate();
    let ticket = rig
        .store
        .create_share_ticket(NewShareTicket {
            workspace_id: rig.tenant.workspace_id,
            channel_id: private,
            owner_id: creator.member_id,
            created_by: creator.member_id,
            created_by_token: None,
            token_hash: hash_secret(secret.as_str()),
            expires_at: Utc::now() + ChronoDuration::hours(1),
            artifact_shas: Vec::new(),
        })
        .await
        .unwrap();
    let authorization = format!("ShareTicket {}", secret.as_str());
    rig.assert_reads(&authorization, thread).await;

    rig.store
        .remove_channel_member(private, creator.member_id)
        .await
        .unwrap();
    rig.assert_refused_as_unknown(&authorization, thread).await;
    assert!(rig.revoked(ticket.id).await);
}
