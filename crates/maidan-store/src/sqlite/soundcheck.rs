//! Soundcheck gate pointer (Cluster 385, Wave 2 #25 remainder). SQLite twin
//! of pg 0088.

use chrono::{DateTime, Utc};
use maidan_types::{
    is_qualifying_pass, resolve_land, soundcheck_standing, standing_land, LandColor, MemberId,
    RecordedSoundcheck, SoundcheckPointer, SoundcheckStanding, SoundcheckStatus, ThreadId,
    SOUNDCHECK_SKILL,
};
use sqlx::{Row, SqlitePool};
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

async fn recorder_has_skill(pool: &SqlitePool, member_id: MemberId) -> Result<bool, StoreError> {
    let skills = super::member_skills::list(pool, member_id).await?;
    Ok(skills.iter().any(|s| s.skill == SOUNDCHECK_SKILL))
}

async fn standing_for(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<SoundcheckStanding, StoreError> {
    let row = sqlx::query(
        "SELECT s.status, s.land, s.artifact_sha, s.recorded_by, s.recorded_at,
                t.owner_id, t.assignee_id
         FROM maidan_thread_soundcheck s
         JOIN maidan_threads t ON t.id = s.thread_id
         WHERE s.thread_id = ?",
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

pub async fn require(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<SoundcheckStanding, StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_thread_soundcheck (thread_id, created_at, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT (thread_id) DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    standing_for(pool, thread_id).await
}

pub async fn set_pointer(
    pool: &SqlitePool,
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
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_thread_soundcheck
            (thread_id, status, land, artifact_sha, recorded_by, recorded_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
            status = excluded.status,
            land = excluded.land,
            artifact_sha = excluded.artifact_sha,
            recorded_by = excluded.recorded_by,
            recorded_at = excluded.recorded_at,
            updated_at = excluded.updated_at",
    )
    .bind(thread_id.0)
    .bind(status.as_str())
    .bind(land.as_str())
    .bind(sha)
    .bind(recorded_by.0)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    standing_for(pool, thread_id).await
}

pub async fn standing(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<SoundcheckStanding, StoreError> {
    standing_for(pool, thread_id).await
}

pub async fn clear(pool: &SqlitePool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_soundcheck WHERE thread_id = ?")
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Cluster 384.2: refuse `closed` when a Soundcheck row exists and is not a
/// qualifying green pass. SQLite twin of the Postgres gate.
pub async fn gate_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT s.status, s.land, s.recorded_by, t.owner_id, t.assignee_id
         FROM maidan_thread_soundcheck s
         JOIN maidan_threads t ON t.id = s.thread_id
         WHERE s.thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(());
    };
    let owner_id = row.get::<Option<Uuid>, _>("owner_id").map(MemberId);
    let assignee_id = row.get::<Option<Uuid>, _>("assignee_id").map(MemberId);
    let recorded_by = row.get::<Option<Uuid>, _>("recorded_by").map(MemberId);
    let status_s: Option<String> = row.get("status");
    let land_s: Option<String> = row.get("land");
    let (status, land, recorded_by) = match (status_s, land_s, recorded_by) {
        (Some(status_s), Some(land_s), Some(recorded_by)) => {
            let status = SoundcheckStatus::parse(&status_s).ok_or_else(|| {
                StoreError::InvalidInput(format!("unknown soundcheck status: {status_s}"))
            })?;
            let land = LandColor::parse(&land_s)
                .ok_or_else(|| StoreError::InvalidInput(format!("unknown land color: {land_s}")))?;
            (status, land, recorded_by)
        }
        _ => {
            return Err(StoreError::Conflict(
                "soundcheck required: no pass recorded".into(),
            ));
        }
    };
    let skilled: i64 = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM maidan_member_skills
            WHERE member_id = ? AND skill = ?
         )",
    )
    .bind(recorded_by.0)
    .bind(SOUNDCHECK_SKILL)
    .fetch_one(&mut **tx)
    .await?;
    let skilled = skilled != 0;
    if is_qualifying_pass(status, land, recorded_by, owner_id, assignee_id, skilled) {
        return Ok(());
    }
    let verdict = standing_land(
        Some(&SoundcheckPointer::new(status, None, land)),
        Some(recorded_by),
        owner_id,
        assignee_id,
        skilled,
        true,
    );
    Err(StoreError::Conflict(format!(
        "soundcheck land is {}: need a green pass from a soundcheck-skilled member who is not the implementer",
        verdict.as_str()
    )))
}
