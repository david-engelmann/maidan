//! Land-gate pointer. One row per thread: presence arms the close-gate; pointer
//! columns stay NULL until a land-gate-skilled member records pass/fail + land.
//! See the SQLite twin.

use chrono::{DateTime, Utc};
use maidan_types::{
    is_qualifying_pass, land_gate_standing, resolve_land, standing_land, LandColor,
    LandGatePointer, LandGateStanding, LandGateStatus, MemberId, RecordedLandGate, ThreadId,
    LAND_GATE_SKILL,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::thread_workers;
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
    Ok(skills.iter().any(|s| s.skill == LAND_GATE_SKILL))
}

async fn standing_for(pool: &PgPool, thread_id: ThreadId) -> Result<LandGateStanding, StoreError> {
    let row = sqlx::query(
        "SELECT s.status, s.land, s.artifact_sha, s.recorded_by, s.recorded_at,
                t.owner_id, t.assignee_id
         FROM maidan_thread_land_gate s
         JOIN maidan_threads t ON t.id = s.thread_id
         WHERE s.thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(land_gate_standing(false, None, None, None, false, false));
    };
    let owner_id = row.get::<Option<Uuid>, _>("owner_id").map(MemberId);
    let assignee_id = row.get::<Option<Uuid>, _>("assignee_id").map(MemberId);
    let recorded_by = row.get::<Option<Uuid>, _>("recorded_by").map(MemberId);
    let status_s: Option<String> = row.get("status");
    let recorded = match (status_s, recorded_by) {
        (Some(status_s), Some(recorded_by)) => {
            let status = LandGateStatus::parse(&status_s).ok_or_else(|| {
                StoreError::InvalidInput(format!("unknown land-gate status: {status_s}"))
            })?;
            let land_s: String = row.get("land");
            let land = LandColor::parse(&land_s)
                .ok_or_else(|| StoreError::InvalidInput(format!("unknown land color: {land_s}")))?;
            Some(RecordedLandGate {
                pointer: LandGatePointer::new(status, row.get("artifact_sha"), land),
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
    // The durable half of "not the implementer". `assignee_id` above is the
    // live holder, which a release clears.
    let worked = match recorded.as_ref().map(|r| r.recorded_by) {
        Some(id) => thread_workers::has_worked(pool, thread_id, id).await?,
        None => false,
    };
    Ok(land_gate_standing(
        true,
        recorded,
        owner_id,
        assignee_id,
        worked,
        skill,
    ))
}

pub async fn require(pool: &PgPool, thread_id: ThreadId) -> Result<LandGateStanding, StoreError> {
    sqlx::query(
        "INSERT INTO maidan_thread_land_gate (thread_id, created_at, updated_at)
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
    status: LandGateStatus,
    artifact_sha: Option<&str>,
    land: Option<LandColor>,
) -> Result<LandGateStanding, StoreError> {
    if !recorder_has_skill(pool, recorded_by).await? {
        return Err(StoreError::InvalidInput(
            "recorder must have the land_gate skill".into(),
        ));
    }
    let sha = artifact_sha_opt(artifact_sha)?;
    let land = resolve_land(status, land);
    sqlx::query(
        "INSERT INTO maidan_thread_land_gate
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

pub async fn standing(pool: &PgPool, thread_id: ThreadId) -> Result<LandGateStanding, StoreError> {
    standing_for(pool, thread_id).await
}

pub async fn clear(pool: &PgPool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_land_gate WHERE thread_id = $1")
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Refuse `closed` when a LandGate row exists and is not a qualifying green
/// pass. Runs on the transition's own tx so it cannot be raced. No row →
/// additive (close as before).
pub async fn gate_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<(), StoreError> {
    let row = sqlx::query(
        "SELECT s.status, s.land, s.recorded_by, t.owner_id, t.assignee_id
         FROM maidan_thread_land_gate s
         JOIN maidan_threads t ON t.id = s.thread_id
         WHERE s.thread_id = $1",
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
            let status = LandGateStatus::parse(&status_s).ok_or_else(|| {
                StoreError::InvalidInput(format!("unknown land-gate status: {status_s}"))
            })?;
            let land = LandColor::parse(&land_s)
                .ok_or_else(|| StoreError::InvalidInput(format!("unknown land color: {land_s}")))?;
            (status, land, recorded_by)
        }
        _ => {
            return Err(StoreError::Conflict(
                "land gate required: no pass recorded".into(),
            ));
        }
    };
    let skilled: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM maidan_member_skills
            WHERE member_id = $1 AND skill = $2
         )",
    )
    .bind(recorded_by.0)
    .bind(LAND_GATE_SKILL)
    .fetch_one(&mut **tx)
    .await?;
    // Did the recorder ever hold this thread? `assignee_id` is the live holder
    // and a release clears it, so without this an implementer could release the
    // claim and then pass their own work through the gate. Read on the
    // enforcing transaction so a concurrent release cannot land between the
    // check and the close.
    let worked = thread_workers::has_worked_in_tx(tx, thread_id, recorded_by).await?;
    if is_qualifying_pass(
        status,
        land,
        recorded_by,
        owner_id,
        assignee_id,
        worked,
        skilled,
    ) {
        return Ok(());
    }
    let verdict = standing_land(
        Some(&LandGatePointer::new(status, None, land)),
        Some(recorded_by),
        owner_id,
        assignee_id,
        worked,
        skilled,
        true,
    );
    Err(StoreError::Conflict(format!(
        "land gate is {}: need a green pass from a land-gate-skilled member who is not the implementer",
        verdict.as_str()
    )))
}
