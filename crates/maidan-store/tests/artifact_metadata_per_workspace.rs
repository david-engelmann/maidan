//! An artifact's bytes are shared across workspaces; what each workspace said
//! about them is its own. Before, the shared row kept the first uploader's
//! `uploaded_by` and the latest uploader's `kind`, so a second tenant uploading
//! the same bytes read back the first tenant's member id and changed what the
//! first tenant saw. On both backends.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{ArtifactKind, MemberKind, NewArtifact, NewMember, NewWorkspace, WorkspaceId};
use sqlx::sqlite::SqlitePoolOptions;

async fn tenant(store: &dyn Store, name: &str) -> (WorkspaceId, maidan_types::MemberId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: name.into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    (ws.id, member.id)
}

async fn run_suite(store: &dyn Store) {
    let (ws_a, alice) = tenant(store, "alpha").await;
    let (ws_b, bob) = tenant(store, "bravo").await;
    let (ws_c, _) = tenant(store, "charlie").await;
    let sha = "a".repeat(64);
    let upload = |kind, mime: &str, by| NewArtifact {
        sha256: sha.clone(),
        size_bytes: 3,
        mime_type: Some(mime.into()),
        kind,
        uploaded_by: Some(by),
    };

    let (as_a, _) = store
        .upsert_artifact_with_event(
            upload(ArtifactKind::Transcript, "text/plain", alice),
            Some(ws_a),
        )
        .await
        .unwrap();
    assert_eq!(as_a.uploaded_by, Some(alice));

    // The same bytes from another tenant.
    let (as_b, _) = store
        .upsert_artifact_with_event(
            upload(ArtifactKind::Screenshot, "image/png", bob),
            Some(ws_b),
        )
        .await
        .unwrap();
    assert_eq!(as_b.uploaded_by, Some(bob), "B must not read A's member id");
    assert_eq!(as_b.kind, ArtifactKind::Screenshot);
    assert_eq!(as_b.mime_type.as_deref(), Some("image/png"));

    let seen_by_a = store.get_artifact_for_workspace(ws_a, &sha).await.unwrap();
    assert_eq!(seen_by_a.uploaded_by, Some(alice));
    assert_eq!(
        seen_by_a.kind,
        ArtifactKind::Transcript,
        "B's upload changed A's view"
    );
    assert_eq!(seen_by_a.mime_type.as_deref(), Some("text/plain"));
    let seen_by_b = store.get_artifact_for_workspace(ws_b, &sha).await.unwrap();
    assert_eq!(seen_by_b.uploaded_by, Some(bob));
    assert_eq!(
        seen_by_a.id, seen_by_b.id,
        "the bytes are still one artifact"
    );

    assert!(matches!(
        store.get_artifact_for_workspace(ws_c, &sha).await,
        Err(StoreError::NotFound)
    ));

    // An artifact no workspace holds a ref to (an unscoped, bypass upload) has
    // no tenant to protect and reads as the shared row from anywhere.
    let unscoped = "b".repeat(64);
    store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: unscoped.clone(),
                size_bytes: 1,
                mime_type: None,
                kind: ArtifactKind::Attachment,
                uploaded_by: None,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .get_artifact_for_workspace(ws_c, &unscoped)
            .await
            .unwrap()
            .kind,
        ArtifactKind::Attachment
    );

    // A bare access grant carries no metadata of its own: it falls back to the
    // shared row for what the bytes are, never for who uploaded them.
    store.record_artifact_ref(ws_c, &sha).await.unwrap();
    let seen_by_c = store.get_artifact_for_workspace(ws_c, &sha).await.unwrap();
    assert_eq!(seen_by_c.uploaded_by, None);
    assert_eq!(seen_by_c.kind, ArtifactKind::Transcript);
}

#[tokio::test]
async fn artifact_metadata_is_per_workspace_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    run_suite(&SqliteStore::new(pool)).await;
}

#[tokio::test]
async fn artifact_metadata_is_per_workspace_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(container) => container,
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
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    run_suite(&PostgresStore::new(pool)).await;
}
