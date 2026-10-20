//! Result-delivery state. SQLite twin of the Postgres module — timestamps are
//! store-bound rfc3339 text, so the `armed_revision` comparison is a string
//! comparison. That is sound because both sides of it are written by this
//! module in one format; it is never compared against a `datetime('now')`
//! column.

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{status, EgressTarget, ResultDelivery, ResultDeliveryId, ThreadId};

const COLS: &str = "id, thread_id, surface, selector, status, external_ref, \
                    armed_revision, delivered_revision, attempts, last_error, \
                    template_warnings, created_at, updated_at";

pub async fn arm(
    pool: &SqlitePool,
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

/// Arm by the raw `(surface, selector)` pair. See the Postgres twin.
pub async fn arm_at(
    pool: &SqlitePool,
    thread_id: ThreadId,
    surface: &str,
    selector: &str,
    revision: DateTime<Utc>,
) -> Result<Option<ResultDelivery>, StoreError> {
    let id = ResultDeliveryId::new();
    let now = Utc::now().to_rfc3339();
    let rev = revision.to_rfc3339();
    let sql = format!(
        "INSERT INTO maidan_result_deliveries
           (id, thread_id, surface, selector, status, armed_revision, created_at, updated_at)
         VALUES (?, ?, ?, ?, '{pending}', ?, ?, ?)
         ON CONFLICT (thread_id, surface, selector) DO UPDATE
           SET status = '{pending}',
               armed_revision = excluded.armed_revision,
               attempts = 0,
               last_error = NULL,
               updated_at = excluded.updated_at
           WHERE excluded.armed_revision > maidan_result_deliveries.armed_revision
         RETURNING {COLS}",
        pending = status::PENDING
    );
    let row = sqlx::query(&sql)
        .bind(id.0)
        .bind(thread_id.0)
        .bind(surface)
        .bind(selector)
        .bind(&rev)
        .bind(&now)
        .bind(&now)
        .fetch_optional(pool)
        .await?;
    Ok(row.as_ref().map(row_to_delivery))
}

pub async fn mark_delivered(
    pool: &SqlitePool,
    id: ResultDeliveryId,
    external_ref: Option<&str>,
    revision: DateTime<Utc>,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{delivered}', external_ref = ?, delivered_revision = ?,
             attempts = attempts + 1, last_error = NULL, updated_at = ?
         WHERE id = ?",
        delivered = status::DELIVERED
    ))
    .bind(external_ref)
    .bind(revision.to_rfc3339())
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_failed(
    pool: &SqlitePool,
    id: ResultDeliveryId,
    error: &str,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{failed}', attempts = attempts + 1, last_error = ?, updated_at = ?
         WHERE id = ?",
        failed = status::FAILED
    ))
    .bind(error)
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_skipped(
    pool: &SqlitePool,
    id: ResultDeliveryId,
    reason: &str,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{skipped}', last_error = ?, updated_at = ?
         WHERE id = ?",
        skipped = status::SKIPPED
    ))
    .bind(reason)
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
    target: &EgressTarget,
) -> Result<Option<ResultDelivery>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_result_deliveries
         WHERE thread_id = ? AND surface = ? AND selector = ?"
    ))
    .bind(thread_id.0)
    .bind(target.surface().as_str())
    .bind(target.selector())
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_delivery))
}

pub async fn list_for_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ResultDelivery>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_result_deliveries
         WHERE thread_id = ?
         ORDER BY surface ASC, selector ASC"
    ))
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_delivery).collect())
}

pub async fn get_by_id(
    pool: &SqlitePool,
    thread_id: ThreadId,
    id: ResultDeliveryId,
) -> Result<Option<ResultDelivery>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_result_deliveries
         WHERE id = ? AND thread_id = ?"
    ))
    .bind(id.0)
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_delivery))
}

/// Reopen as pending for operator replay. Leaves `armed_revision` and
/// `external_ref` alone — this is not a new result, and the handle to edit
/// is still the one we created.
pub async fn prepare_replay(
    pool: &SqlitePool,
    thread_id: ThreadId,
    id: ResultDeliveryId,
) -> Result<Option<ResultDelivery>, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{pending}', last_error = NULL, updated_at = ?
         WHERE id = ? AND thread_id = ?
         RETURNING {COLS}",
        pending = status::PENDING
    ))
    .bind(&now)
    .bind(id.0)
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_delivery))
}

/// Record what the pull request a change delivered lacks against the
/// producer's template, replacing what an earlier round recorded. Empty
/// clears it. Kept apart from `last_error`, which means "did not land".
pub async fn set_template_warnings(
    pool: &SqlitePool,
    thread_id: ThreadId,
    target: &EgressTarget,
    warnings: &[String],
) -> Result<(), StoreError> {
    let codes = (!warnings.is_empty()).then(|| warnings.join(","));
    sqlx::query(
        "UPDATE maidan_result_deliveries SET template_warnings = ?4
         WHERE thread_id = ?1 AND surface = ?2 AND selector = ?3",
    )
    .bind(thread_id.0)
    .bind(target.surface().as_str())
    .bind(target.selector())
    .bind(codes)
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_delivery(row: &sqlx::sqlite::SqliteRow) -> ResultDelivery {
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
        template_warnings: row
            .get::<Option<String>, _>("template_warnings")
            .map(|codes| codes.split(',').map(str::to_string).collect())
            .unwrap_or_default(),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

/// The commit last delivered to a pull request by the change flow; see
/// `Store::delivered_change_head`. A `github_branch` row's selector is
/// `owner/name@branch` and its handle `<commit_sha>#<pull_number>`.
pub async fn delivered_change_head(
    pool: &sqlx::SqlitePool,
    workspace_id: maidan_types::WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Option<String>, StoreError> {
    let handle: Option<String> = sqlx::query_scalar(
        "SELECT rd.external_ref FROM maidan_result_deliveries rd
         JOIN maidan_threads t ON t.id = rd.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?1
           AND rd.surface = 'github_branch' AND rd.status = 'delivered'
           AND lower(substr(rd.selector, 1, length(?2) + 1)) = lower(?2) || '@'
           AND rd.external_ref LIKE '%#' || ?3
         ORDER BY rd.updated_at DESC
         LIMIT 1",
    )
    .bind(workspace_id.0)
    .bind(repo)
    .bind(pull_number.to_string())
    .fetch_optional(pool)
    .await?
    .flatten();
    Ok(handle.and_then(|handle| {
        let (sha, number) = handle.split_once('#')?;
        (number == pull_number.to_string() && maidan_types::is_git_sha(sha))
            .then(|| sha.to_string())
    }))
}
