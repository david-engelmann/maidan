use chrono::{DateTime, Utc};
use maidan_types::{Artifact, ArtifactId, ArtifactKind, MemberId, NewArtifact};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn upsert(pool: &PgPool, new: NewArtifact) -> Result<Artifact, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_artifacts (id, sha256, size_bytes, mime_type, kind, uploaded_by)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (sha256) DO UPDATE
             SET mime_type = COALESCE(EXCLUDED.mime_type, maidan_artifacts.mime_type),
                 kind = EXCLUDED.kind
         RETURNING id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at",
    )
    .bind(id)
    .bind(&new.sha256)
    .bind(new.size_bytes)
    .bind(new.mime_type.as_deref())
    .bind(new.kind.as_str())
    .bind(new.uploaded_by.map(|m| m.0))
    .fetch_one(pool)
    .await?;
    row_to_artifact(&row)
}

pub async fn get_by_sha(pool: &PgPool, sha256: &str) -> Result<Artifact, StoreError> {
    let row = sqlx::query(
        "SELECT id, sha256, size_bytes, mime_type, kind, uploaded_by, created_at, tombstoned_at
         FROM maidan_artifacts WHERE sha256 = $1",
    )
    .bind(sha256)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_artifact(&row)
}

fn row_to_artifact(row: &sqlx::postgres::PgRow) -> Result<Artifact, StoreError> {
    let kind: String = row.get("kind");
    let kind = ArtifactKind::parse(&kind).ok_or_else(|| {
        StoreError::InvalidInput(format!("unknown artifact kind in database: {kind}"))
    })?;
    Ok(Artifact {
        id: ArtifactId(row.get::<Uuid, _>("id")),
        sha256: row.get("sha256"),
        size_bytes: row.get("size_bytes"),
        mime_type: row.get("mime_type"),
        kind,
        uploaded_by: row.get::<Option<Uuid>, _>("uploaded_by").map(MemberId),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}
