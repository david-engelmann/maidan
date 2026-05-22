use chrono::{DateTime, Utc};
use maidan_types::{Artifact, ArtifactId, MemberId, NewArtifact};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn upsert(pool: &SqlitePool, new: NewArtifact) -> Result<Artifact, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let row = sqlx::query(
        "INSERT INTO maidan_artifacts (id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (sha256) DO UPDATE
             SET mime_type = COALESCE(excluded.mime_type, maidan_artifacts.mime_type),
                 kind = excluded.kind
         RETURNING id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at",
    )
    .bind(id)
    .bind(&new.sha256)
    .bind(new.size_bytes)
    .bind(new.mime_type.as_deref())
    .bind(&new.kind)
    .bind(new.uploaded_by.map(|m| m.0))
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_artifact(&row))
}

pub async fn get_by_sha(pool: &SqlitePool, sha256: &str) -> Result<Artifact, StoreError> {
    let row = sqlx::query(
        "SELECT id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at
         FROM maidan_artifacts WHERE sha256 = ?",
    )
    .bind(sha256)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_artifact(&row))
}

fn row_to_artifact(row: &sqlx::sqlite::SqliteRow) -> Artifact {
    Artifact {
        id: ArtifactId(row.get::<Uuid, _>("id")),
        sha256: row.get("sha256"),
        size_bytes: row.get("size_bytes"),
        mime_type: row.get("mime_type"),
        kind: row.get("kind"),
        uploaded_by: row.get::<Option<Uuid>, _>("uploaded_by").map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    }
}
