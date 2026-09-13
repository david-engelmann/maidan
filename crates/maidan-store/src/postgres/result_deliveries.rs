//! Result-delivery state (Cluster 379.1). See the SQLite twin and
//! `migrations/postgres/0085_result_deliveries.sql`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{status, EgressTarget, ResultDelivery, ResultDeliveryId, ThreadId};

const COLS: &str = "id, thread_id, surface, selector, status, external_ref, \
                    armed_revision, delivered_revision, attempts, last_error, \
                    created_at, updated_at";

/// Arm `(thread, target)` for `revision`, returning the row when *this* caller
/// won the right to deliver it.
///
/// `None` means "not yours": either another replica already armed this exact
/// revision, or the row has seen a revision at least this new. That is the whole
/// dedup — every replica's notification router calls this for the same event,
/// and the unique index makes exactly one of them win.
///
/// A won row keeps its `external_ref`, so a re-review becomes an edit of the
/// object the first delivery created rather than a second comment.
pub async fn arm(
    pool: &PgPool,
    thread_id: ThreadId,
    target: &EgressTarget,
    revision: DateTime<Utc>,
) -> Result<Option<ResultDelivery>, StoreError> {
    arm_at(
        pool,
        thread_id,
        target.surface().as_str(),
        &target.selector(),
        revision,
    )
    .await
}

/// Arm by the raw `(surface, selector)` pair. Used for a skip whose destination
/// this build cannot form an [`EgressTarget`] for — an unknown surface, or a
/// known one with unusable detail — so the skip is still a row the producer
/// can read.
pub async fn arm_at(
    pool: &PgPool,
    thread_id: ThreadId,
    surface: &str,
    selector: &str,
    revision: DateTime<Utc>,
) -> Result<Option<ResultDelivery>, StoreError> {
    let id = ResultDeliveryId::new();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_result_deliveries
           (id, thread_id, surface, selector, status, armed_revision, created_at, updated_at)
         VALUES ($1, $2, $3, $4, '{pending}', $5, now(), now())
         ON CONFLICT (thread_id, surface, selector) DO UPDATE
           SET status = '{pending}',
               armed_revision = $5,
               attempts = 0,
               last_error = NULL,
               updated_at = now()
           WHERE $5 > maidan_result_deliveries.armed_revision
         RETURNING {COLS}",
        pending = status::PENDING
    ))
    .bind(id.0)
    .bind(thread_id.0)
    .bind(surface)
    .bind(selector)
    .bind(revision)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_delivery))
}

/// Record a delivery that landed: `delivered`, the handle to edit next time, and
/// the revision that actually reached the surface.
///
/// `external_ref` is `None` when the surface accepted the message but handed back
/// no usable handle (Cluster 378.2) — still a delivery, just not an addressable
/// one, so the next revision posts instead of editing.
pub async fn mark_delivered(
    pool: &PgPool,
    id: ResultDeliveryId,
    external_ref: Option<&str>,
    revision: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{delivered}', external_ref = $2, delivered_revision = $3,
             attempts = attempts + 1, last_error = NULL, updated_at = now()
         WHERE id = $1",
        delivered = status::DELIVERED
    ))
    .bind(id.0)
    .bind(external_ref)
    .bind(revision)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record that delivery failed for good (the transport gave up). `external_ref`
/// and `delivered_revision` are left alone: whatever was delivered *before* this
/// attempt is still out there and still editable.
pub async fn mark_failed(
    pool: &PgPool,
    id: ResultDeliveryId,
    error: &str,
) -> Result<(), StoreError> {
    sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{failed}', attempts = attempts + 1, last_error = $2, updated_at = now()
         WHERE id = $1",
        failed = status::FAILED
    ))
    .bind(id.0)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a target we deliberately did not deliver to — an unknown surface, or
/// one the workspace has not blessed. **Not an error:** `docs/Result Delivery.md`
/// makes "delivered nowhere" a normal outcome, so the disposition is recorded
/// with its reason instead of being dropped, and the producer can read it back.
pub async fn mark_skipped(
    pool: &PgPool,
    id: ResultDeliveryId,
    reason: &str,
) -> Result<(), StoreError> {
    sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{skipped}', last_error = $2, updated_at = now()
         WHERE id = $1",
        skipped = status::SKIPPED
    ))
    .bind(id.0)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(
    pool: &PgPool,
    thread_id: ThreadId,
    target: &EgressTarget,
) -> Result<Option<ResultDelivery>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_result_deliveries
         WHERE thread_id = $1 AND surface = $2 AND selector = $3"
    ))
    .bind(thread_id.0)
    .bind(target.surface().as_str())
    .bind(target.selector())
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_delivery))
}

/// Every target this thread's result has been aimed at, for the delivery-status
/// API. Ordered so the listing is stable rather than insertion-dependent.
pub async fn list_for_thread(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<ResultDelivery>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_result_deliveries
         WHERE thread_id = $1
         ORDER BY surface ASC, selector ASC"
    ))
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_delivery).collect())
}

fn row_to_delivery(row: &sqlx::postgres::PgRow) -> ResultDelivery {
    ResultDelivery {
        id: ResultDeliveryId(row.get("id")),
        thread_id: ThreadId(row.get("thread_id")),
        surface: row.get("surface"),
        selector: row.get("selector"),
        status: row.get("status"),
        external_ref: row.get("external_ref"),
        armed_revision: row.get("armed_revision"),
        delivered_revision: row.get("delivered_revision"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
