//! Capability-ticket lifecycle and tenant/artifact isolation on both stores.

use chrono::{Duration as ChronoDuration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ArtifactKind, MemberKind, NewArtifact, NewChannel, NewMember, NewShareTicket, NewWorkspace,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
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
            eprintln!("skipping: docker unavailable ({err})");
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
    run_suite(&PostgresStore::new(pool)).await;
}
