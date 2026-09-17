//! Durable resume point for a tap projector.
//!
//! The search tap re-walked the whole log from id 0 on every start, resubscribe
//! and `Lagged`. This is where it remembers instead.

use sqlx::{Row, SqlitePool};

use crate::StoreError;

/// Where `surface` last finished projecting. `0` = never run.
pub async fn get(pool: &SqlitePool, surface: &str) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT last_event_id FROM maidan_tap_cursor WHERE surface = ?")
        .bind(surface)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<i64, _>("last_event_id")).unwrap_or(0))
}

/// Advance `surface` to `last_event_id`.
///
/// Monotonic: a lower value is ignored rather than written. Two replicas both
/// run the tap, and a slower one finishing its page later must not drag the
/// cursor backwards — that would re-project history on the next restart, which
/// is the cost this table exists to remove.
pub async fn set(pool: &SqlitePool, surface: &str, last_event_id: i64) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_tap_cursor (surface, last_event_id, updated_at)
         VALUES (?, ?, datetime('now'))
         ON CONFLICT (surface) DO UPDATE
           SET last_event_id = MAX(maidan_tap_cursor.last_event_id, excluded.last_event_id),
               updated_at = excluded.updated_at",
    )
    .bind(surface)
    .bind(last_event_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Forget the resume point, so the next backfill re-walks from genesis.
///
/// The rebuild path: a chain break means the projection is not trustworthy, and
/// resuming past it would preserve exactly the divergence that was detected.
pub async fn clear(pool: &SqlitePool, surface: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_tap_cursor WHERE surface = ?")
        .bind(surface)
        .execute(pool)
        .await?;
    Ok(())
}
