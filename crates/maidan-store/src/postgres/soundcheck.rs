//! Soundcheck gate pointer (Cluster 385, Wave 2 #25 remainder). One row
//! per thread: presence arms the close-gate; pointer columns stay NULL
//! until a soundcheck-skilled member records pass/fail + land.
//! See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{
    resolve_land, soundcheck_standing, LandColor, MemberId, RecordedSoundcheck, SoundcheckPointer,
    SoundcheckStanding, SoundcheckStatus, ThreadId, SOUNDCHECK_SKILL,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn artifact_sha_opt(raw: Option<&str>) -> Result<Option<String>, StoreError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) if s.len() > 128 => Err(StoreError::InvalidInput(
            "artifact_sha must be at most 128 bytes".into(),
        )),
        Some(s) => Ok(Some(s.to_string())),
    }
}

async fn recorder_has_skill(pool: &PgPool, member_id: MemberId) -> Result<bool, StoreError> {
    let skills = super::member_skills::list(pool, member_id).await?;
    Ok(skills.iter().any(|s| s.skill == SOUNDCHECK_SKILL))
}

async fn standing_for(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<SoundcheckStanding, StoreError> {
    let row = sqlx::query(
        "SELECT s.status, s.land, s.artifact_sha, s.recorded_by, s.recorded_at,
                t.owner_id, t.assignee_id
         FROM maidan_thread_soundcheck s
         JOIN maidan_threads t ON t.id = s.thread_id
         WHERE s.thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(soundcheck_standing(false, None, None, None, false));
    };
    let owner_id = row.get::<Option<Uuid>, _>("owner_id").map(MemberId);
    let assignee_id = row.get::<Option<Uuid>, _>("assignee_id").map(MemberId);
    let recorded_by = row.get::<Option<Uuid>, _>("recorded_by").map(MemberId);
    let status_s: Option<String> = row.get("status");
    let recorded = match (status_s, recorded_by) {
        (Some(status_s), Some(recorded_by)) => {
            let status = SoundcheckStatus::parse(&status_s).ok_or_else(|| {
                StoreError::InvalidInput(format!("unknown soundcheck status: {status_s}"))
            })?;
            let land_s: String = row.get("land");
            let land = LandColor::parse(&land_s)
                .ok_or_else(|| StoreError::InvalidInput(format!("unknown land color: {land_s}")))?;
            Some(RecordedSoundcheck {
                pointer: SoundcheckPointer::new(status, row.get("artifact_sha"), land),
                recorded_by,
                recorded_at: row.get::<DateTime<Utc>, _>("recorded_at"),
            })
        }
        _ => None,
    };
    let skill = match recorded.as_ref().map(|r| r.recorded_by) {
        Some(id) => recorder_has_skill(pool, id).await?,
        None => false,
    };
    Ok(soundcheck_standing(
        true,
        recorded,
        owner_id,
        assignee_id,
        skill,
    ))
}

pub async fn require(pool: &PgPool, thread_id: ThreadId) -> Result<SoundcheckStanding, StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_soundcheck (thread_id, created_at, updated_at)
         VALUES ($1, NOW(), NOW())
         ON CONFLICT (thread_id) DO NOTHING",
    )
    .bind(thread_id.0)
    .execute(pool)
    .await?;
    standing_for(pool, thread_id).await
}

pub async fn set_pointer(
    pool: &PgPool,
    thread_id: ThreadId,
    recorded_by: MemberId,
    status: SoundcheckStatus,
    artifact_sha: Option<&str>,
    land: Option<LandColor>,
) -> Result<SoundcheckStanding, StoreError> {
    if !recorder_has_skill(pool, recorded_by).await? {
        return Err(StoreError::InvalidInput(
            "recorder must have the soundcheck skill".into(),
        ));
    }
    let sha = artifact_sha_opt(artifact_sha)?;
    let land = resolve_land(status, land);
    sqlx::query(
        "INSERT INTO maidan_thread_soundcheck
            (thread_id, status, land, artifact_sha, recorded_by, recorded_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW(), NOW())
         ON CONFLICT (thread_id) DO UPDATE SET
            status = excluded.status,
            land = excluded.land,
            artifact_sha = excluded.artifact_sha,
            recorded_by = excluded.recorded_by,
            recorded_at = NOW(),
            updated_at = NOW()",
    )
    .bind(thread_id.0)
    .bind(status.as_str())
    .bind(land.as_str())
    .bind(sha)
    .bind(recorded_by.0)
    .execute(pool)
    .await?;
    standing_for(pool, thread_id).await
}

pub async fn standing(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<SoundcheckStanding, StoreError> {
    standing_for(pool, thread_id).await
}

pub async fn clear(pool: &PgPool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_soundcheck WHERE thread_id = $1")
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
