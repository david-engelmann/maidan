//! Migration 0135: a member handle is unique regardless of case, and an
//! upgraded database renames every case-only duplicate but the oldest.

use chrono::{DateTime, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::*;
use sqlx::sqlite::SqlitePoolOptions;
use uuid::Uuid;

async fn spawn() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign keys");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::for_tests(pool)
}

fn suffix(id: MemberId) -> String {
    // SQLite stores the id as a 16-byte blob; the migration appends hex(id).
    id.0.as_bytes().iter().map(|b| format!("{b:02X}")).collect()
}

#[tokio::test]
async fn migration_renames_case_only_duplicates_and_keeps_the_oldest() {
    let store = spawn().await;
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "handles".into(),
        })
        .await
        .unwrap();
    let other = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .unwrap();
    let kept_elsewhere = store
        .create_member(NewMember {
            workspace_id: other.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();

    // The index is already in place on a fresh database. Drop it so the
    // pre-0135 shape (case-sensitive uniqueness) can hold both spellings,
    // then run the migration again.
    sqlx::query("DROP INDEX idx_members_handle_ci")
        .execute(store.pool())
        .await
        .unwrap();

    let newer = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let bob = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();

    let oldest_id = Uuid::now_v7();
    let long_ago = DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    sqlx::query(
        "INSERT INTO maidan_members
            (id, workspace_id, handle, display_name, kind, created_at, updated_at)
         VALUES (?, ?, 'Alice', NULL, 'human', ?, ?)",
    )
    .bind(oldest_id)
    .bind(workspace.id.0)
    .bind(long_ago)
    .bind(long_ago)
    .execute(store.pool())
    .await
    .unwrap();

    // A handle that is already the form the rename would pick. The migration
    // must not land on it.
    let taken_form = format!("alice~{}", suffix(newer.id));
    let taken = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: taken_form.clone(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();

    sqlx::raw_sql(include_str!(
        "../../../migrations/sqlite/0135_member_handle_ci.sql"
    ))
    .execute(store.pool())
    .await
    .unwrap();

    let oldest = store.get_member(MemberId(oldest_id)).await.unwrap();
    assert_eq!(oldest.handle, "Alice", "the oldest spelling is kept");
    let renamed = store.get_member(newer.id).await.unwrap();
    assert_eq!(
        renamed.handle,
        format!("{taken_form}~{}", suffix(newer.id)),
        "a taken suffix is doubled rather than colliding"
    );
    assert_eq!(store.get_member(bob.id).await.unwrap().handle, "bob");
    assert_eq!(store.get_member(taken.id).await.unwrap().handle, taken_form);
    assert_eq!(
        store.get_member(kept_elsewhere.id).await.unwrap().handle,
        "alice",
        "another workspace is a different namespace"
    );

    assert_eq!(
        store
            .get_member_by_handle(workspace.id, "aLiCe")
            .await
            .unwrap()
            .id,
        MemberId(oldest_id)
    );
    let again = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "ALICE".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await;
    assert!(
        matches!(again, Err(StoreError::Conflict(_))),
        "the index refuses a case-only duplicate, got {again:?}"
    );
}
