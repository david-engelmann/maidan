//! Capability-ticket lifecycle and tenant/artifact isolation on both stores.

use chrono::{Duration as ChronoDuration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ArtifactKind, ChannelMemberRole, MemberKind, NewApiToken, NewArtifact, NewChannel, NewMember,
    NewShareTicket, NewWorkspace,
};

async fn sqlite() -> SqliteStore {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::for_tests(pool)
}

/// The token a ticket was minted with is stored and read back by every path
/// that returns a ticket, so each use can resolve it again.
async fn a_ticket_records_the_token_that_minted_it(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "token-bearing".into(),
        })
        .await
        .expect("workspace");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "minter".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "open".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("channel");
    let token = store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: "c".repeat(64),
            label: None,
            capabilities: vec!["token:admin".into()],
            expires_at: None,
        })
        .await
        .expect("token");
    let now = Utc::now();
    let mint = |created_by_token, hash: &str| NewShareTicket {
        workspace_id: ws.id,
        channel_id: channel.id,
        owner_id: member.id,
        created_by: member.id,
        created_by_token,
        token_hash: hash.repeat(64),
        expires_at: now + ChronoDuration::hours(1),
        artifact_shas: Vec::new(),
    };
    let with_token = store
        .create_share_ticket(mint(Some(token.id), "d"))
        .await
        .expect("ticket with a token");
    let without_token = store
        .create_share_ticket(mint(None, "e"))
        .await
        .expect("ticket without a token");
    assert_eq!(with_token.created_by_token, Some(token.id));
    assert_eq!(without_token.created_by_token, None);
    for (hash, expected) in [("d", Some(token.id)), ("e", None)] {
        let resolved = store
            .resolve_share_ticket(&hash.repeat(64), now)
            .await
            .expect("resolve");
        assert_eq!(resolved.created_by_token, expected);
    }
    assert_eq!(
        store
            .get_share_ticket(with_token.id)
            .await
            .expect("get")
            .created_by_token,
        Some(token.id)
    );
    let listed = store.list_share_tickets(ws.id).await.expect("list");
    assert!(listed
        .iter()
        .any(|ticket| ticket.id == with_token.id && ticket.created_by_token == Some(token.id)));
}

async fn run_suite(store: &dyn Store) {
    a_ticket_records_the_token_that_minted_it(store).await;
    let ws = store
        .create_workspace(NewWorkspace {
            name: "incident owner".into(),
        })
        .await
        .expect("workspace");
    let other_ws = store
        .create_workspace(NewWorkspace {
            name: "other tenant".into(),
        })
        .await
        .expect("other workspace");
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("owner");
    let other_member = store
        .create_member(NewMember {
            workspace_id: other_ws.id,
            handle: "other".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("other member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "incident".into(),
            topic: None,
            private: true,
        })
        .await
        .expect("channel");
    let other_channel = store
        .create_channel(NewChannel {
            workspace_id: other_ws.id,
            name: "private-other".into(),
            topic: None,
            private: true,
        })
        .await
        .expect("other channel");
    store
        .add_channel_member(channel.id, owner.id, ChannelMemberRole::Member)
        .await
        .expect("owner joins the private channel");

    let allowed_sha = "1".repeat(64);
    let same_workspace_unshared_sha = "2".repeat(64);
    let other_workspace_sha = "3".repeat(64);
    for (sha, uploader, ref_workspace) in [
        (&allowed_sha, owner.id, ws.id),
        (&same_workspace_unshared_sha, owner.id, ws.id),
        (&other_workspace_sha, other_member.id, other_ws.id),
    ] {
        store
            .upsert_artifact(NewArtifact {
                sha256: sha.clone(),
                size_bytes: 1,
                mime_type: Some("application/octet-stream".into()),
                filename: None,
                kind: ArtifactKind::Attachment,
                uploaded_by: Some(uploader),
            })
            .await
            .expect("artifact");
        store
            .record_artifact_ref(ref_workspace, sha)
            .await
            .expect("artifact ref");
    }

    let now = Utc::now();
    let token_hash = "a".repeat(64);
    let ticket = store
        .create_share_ticket(NewShareTicket {
            workspace_id: ws.id,
            channel_id: channel.id,
            owner_id: owner.id,
            created_by: owner.id,
            created_by_token: None,
            token_hash: token_hash.clone(),
            expires_at: now + ChronoDuration::hours(47),
            artifact_shas: vec![allowed_sha.clone(), allowed_sha.clone()],
        })
        .await
        .expect("ticket");
    assert_eq!(ticket.workspace_id, ws.id);
    assert_eq!(ticket.channel_id, channel.id);
    assert_eq!(ticket.owner_id, owner.id);
    assert_eq!(ticket.token_hash, token_hash);
    assert_eq!(
        store
            .list_share_ticket_artifacts(ticket.id)
            .await
            .expect("ticket artifacts"),
        vec![allowed_sha.clone()],
        "duplicate artifact grants are normalized"
    );
    assert_eq!(
        store
            .resolve_share_ticket(&token_hash, now)
            .await
            .expect("resolve")
            .id,
        ticket.id
    );
    assert!(store
        .resolve_share_ticket(&"f".repeat(64), now)
        .await
        .is_err());
    assert!(store
        .share_ticket_allows_artifact(ticket.id, &allowed_sha, now)
        .await
        .expect("allows selected"));
    assert!(!store
        .share_ticket_allows_artifact(ticket.id, &same_workspace_unshared_sha, now)
        .await
        .expect("denies unselected"));
    assert!(store
        .resolve_share_ticket(&token_hash, now + ChronoDuration::hours(48))
        .await
        .is_err());

    let before_failed_create = store
        .list_share_tickets(ws.id)
        .await
        .expect("list before")
        .len();
    let cross_tenant_artifact = store
        .create_share_ticket(NewShareTicket {
            workspace_id: ws.id,
            channel_id: channel.id,
            owner_id: owner.id,
            created_by: owner.id,
            created_by_token: None,
            token_hash: "b".repeat(64),
            expires_at: now + ChronoDuration::hours(1),
            artifact_shas: vec![other_workspace_sha],
        })
        .await;
    assert!(cross_tenant_artifact.is_err());
    assert_eq!(
        store
            .list_share_tickets(ws.id)
            .await
            .expect("list after")
            .len(),
        before_failed_create,
        "ticket and artifact scope roll back together"
    );

    assert!(store
        .create_share_ticket(NewShareTicket {
            workspace_id: ws.id,
            channel_id: other_channel.id,
            owner_id: owner.id,
            created_by: owner.id,
            created_by_token: None,
            token_hash: "c".repeat(64),
            expires_at: now + ChronoDuration::hours(1),
            artifact_shas: Vec::new(),
        })
        .await
        .is_err());
    assert!(store
        .create_share_ticket(NewShareTicket {
            workspace_id: ws.id,
            channel_id: channel.id,
            owner_id: other_member.id,
            created_by: owner.id,
            created_by_token: None,
            token_hash: "d".repeat(64),
            expires_at: now + ChronoDuration::hours(1),
            artifact_shas: Vec::new(),
        })
        .await
        .is_err());
    assert!(store
        .create_share_ticket(NewShareTicket {
            workspace_id: ws.id,
            channel_id: channel.id,
            owner_id: owner.id,
            created_by: owner.id,
            created_by_token: None,
            token_hash: "e".repeat(64),
            expires_at: now + ChronoDuration::hours(49),
            artifact_shas: Vec::new(),
        })
        .await
        .is_err());
    assert!(store
        .list_share_tickets(other_ws.id)
        .await
        .expect("other list")
        .is_empty());

    assert!(store
        .revoke_share_ticket(ws.id, ticket.id)
        .await
        .expect("revoke"));
    assert!(!store
        .revoke_share_ticket(ws.id, ticket.id)
        .await
        .expect("revoke twice"));
    assert!(store.resolve_share_ticket(&token_hash, now).await.is_err());
    assert!(!store
        .share_ticket_allows_artifact(ticket.id, &allowed_sha, now)
        .await
        .expect("revoked artifact denial"));
    assert!(store
        .get_share_ticket(ticket.id)
        .await
        .expect("get")
        .revoked_at
        .is_some());
}

/// A ticket resolves until the instant it expires, and not at that instant.
///
/// Create judges the deadline against its own clock, taken when the insert
/// runs. A deadline computed earlier and only 50 ms ahead is already past
/// when a slow runner reaches the insert, so minting is refused. The
/// deadline here is long enough that the runner cannot arrive late. The
/// check that the ticket is valid one millisecond before expiry, and gone
/// at expiry, uses the instant passed to resolve, not the wall clock. The
/// one-millisecond lower bound on an expiry is
/// `an_expiry_is_accepted_only_past_the_database_resolution`.
async fn a_ticket_is_valid_until_the_instant_it_expires(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "short-lived".into(),
        })
        .await
        .expect("workspace");
    let owner = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "owner".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("owner");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "brief".into(),
            topic: None,
            private: true,
        })
        .await
        .expect("channel");
    store
        .add_channel_member(channel.id, owner.id, ChannelMemberRole::Member)
        .await
        .expect("owner joins the private channel");

    let token_hash = "b".repeat(64);
    let before = Utc::now();
    let expires_at = before + ChronoDuration::seconds(60);
    let ticket = store
        .create_share_ticket(NewShareTicket {
            workspace_id: ws.id,
            channel_id: channel.id,
            owner_id: owner.id,
            created_by: owner.id,
            created_by_token: None,
            token_hash: token_hash.clone(),
            expires_at,
            artifact_shas: Vec::new(),
        })
        .await
        .expect("a ticket is minted");
    assert!(ticket.created_at >= before && ticket.created_at < ticket.expires_at);

    let just_before = ticket.expires_at - ChronoDuration::milliseconds(1);
    assert_eq!(
        store
            .resolve_share_ticket(&token_hash, just_before)
            .await
            .expect("valid until its expiry")
            .id,
        ticket.id
    );
    assert!(matches!(
        store
            .resolve_share_ticket(&token_hash, ticket.expires_at)
            .await,
        Err(StoreError::NotFound)
    ));
}

/// A ticket reads every thread and message of its channel, so minting one for
/// the shared DM channel, or for a private channel the creator is not in, is
/// refused. A public channel, and a private one the creator belongs to, still
/// work.
async fn a_ticket_cannot_open_the_dm_channel_or_a_private_channel_its_creator_is_not_in(
    store: &dyn Store,
) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "tickets".into(),
        })
        .await
        .expect("workspace");
    let member = |handle: &str| NewMember {
        workspace_id: ws.id,
        handle: handle.into(),
        display_name: None,
        kind: MemberKind::Human,
    };
    let admin = store.create_member(member("admin")).await.expect("admin");
    let insider = store
        .create_member(member("insider"))
        .await
        .expect("insider");
    let peer = store.create_member(member("peer")).await.expect("peer");
    let channel = |name: &str, private: bool| NewChannel {
        workspace_id: ws.id,
        name: name.into(),
        topic: None,
        private,
    };
    let public = store
        .create_channel(channel("public", false))
        .await
        .expect("public");
    let private = store
        .create_channel(channel("private", true))
        .await
        .expect("private");
    store
        .add_channel_member(private.id, insider.id, ChannelMemberRole::Member)
        .await
        .expect("insider joins");
    // Opening a conversation is what creates the workspace's DM channel.
    let dm = store
        .open_dm_conversation(ws.id, insider.id, peer.id)
        .await
        .expect("dm");
    let dm_channel = store
        .get_thread(dm.thread_id)
        .await
        .expect("dm thread")
        .channel_id;
    assert_eq!(
        store
            .get_channel(dm_channel)
            .await
            .expect("dm channel")
            .name,
        maidan_types::DM_CHANNEL_NAME
    );

    let mint = |channel_id, created_by, hash: &str| NewShareTicket {
        workspace_id: ws.id,
        channel_id,
        owner_id: created_by,
        created_by,
        created_by_token: None,
        token_hash: hash.repeat(64),
        expires_at: Utc::now() + ChronoDuration::hours(1),
        artifact_shas: Vec::new(),
    };

    for creator in [admin.id, insider.id, peer.id] {
        let refused = store
            .create_share_ticket(mint(dm_channel, creator, "1"))
            .await;
        assert!(
            matches!(&refused, Err(StoreError::InvalidInput(m)) if m.contains("direct messages")),
            "the DM channel is never shared: {refused:?}"
        );
    }
    let outsider = store
        .create_share_ticket(mint(private.id, admin.id, "2"))
        .await;
    assert!(
        matches!(&outsider, Err(StoreError::InvalidInput(m)) if m.contains("private channel")),
        "an admin outside a private channel cannot share it: {outsider:?}"
    );
    assert!(
        store
            .create_share_ticket(mint(private.id, insider.id, "3"))
            .await
            .is_ok(),
        "a member of the private channel can share it"
    );
    assert!(
        store
            .create_share_ticket(mint(public.id, admin.id, "4"))
            .await
            .is_ok(),
        "a public channel can be shared by anyone who may mint"
    );
    assert_eq!(
        store.list_share_tickets(ws.id).await.expect("list").len(),
        2,
        "a refused ticket leaves no row"
    );
}

#[tokio::test]
async fn a_ticket_cannot_open_the_dm_channel_or_a_private_channel_its_creator_is_not_in_sqlite() {
    a_ticket_cannot_open_the_dm_channel_or_a_private_channel_its_creator_is_not_in(&sqlite().await)
        .await;
}

#[tokio::test]
async fn a_ticket_is_valid_until_the_instant_it_expires_sqlite() {
    a_ticket_is_valid_until_the_instant_it_expires(&sqlite().await).await;
}

#[tokio::test]
async fn share_ticket_lifecycle_sqlite() {
    run_suite(&sqlite().await).await;
}

#[tokio::test]
async fn share_ticket_lifecycle_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool);
    run_suite(&store).await;
    a_ticket_is_valid_until_the_instant_it_expires(&store).await;
    a_ticket_cannot_open_the_dm_channel_or_a_private_channel_its_creator_is_not_in(&store).await;
}
