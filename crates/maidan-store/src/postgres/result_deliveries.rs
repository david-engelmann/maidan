//! Result-delivery state. See the SQLite twin and
//! `migrations/postgres/0085_result_deliveries.sql`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use super::events;
use crate::StoreError;
use maidan_types::{
    status, EgressTarget, Event, ResultDelivery, ResultDeliveryId, StoredEvent, ThreadId,
};

const COLS: &str = "id, thread_id, surface, selector, status, external_ref, \
                    armed_revision, delivered_revision, attempts, last_error, \
                    template_warnings, created_at, updated_at";

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

/// Record a delivery that landed: `delivered`, the handle to edit next time,
/// and the revision that actually reached the surface.
///
/// `external_ref` is `None` when the surface accepted the message but handed
/// back no usable handle — still a delivery, just not an addressable one, so
/// the next revision posts instead of editing.
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
) -> Result<Option<StoredEvent>, StoreError> {
    mark_undelivered(pool, id, status::FAILED, error, true).await
}

/// Record a target we deliberately did not deliver to — an unknown surface, or
/// one the workspace has not blessed. **Not an error:** `docs/Result Delivery.md`
/// makes "delivered nowhere" a normal outcome, so the disposition is recorded
/// with its reason instead of being dropped, and the producer can read it back.
pub async fn mark_skipped(
    pool: &PgPool,
    id: ResultDeliveryId,
    reason: &str,
) -> Result<Option<StoredEvent>, StoreError> {
    mark_undelivered(pool, id, status::SKIPPED, reason, false).await
}

/// Record a target as `failed` or `skipped` and, when the row moved into that
/// status, append `ResultDeliveryFailed` in the same transaction, so a relay
/// hears each outcome once and never hears one that was not recorded. The row
/// is locked first, so two replicas recording the same row say it once.
async fn mark_undelivered(
    pool: &PgPool,
    id: ResultDeliveryId,
    to: &str,
    why: &str,
    count_attempt: bool,
) -> Result<Option<StoredEvent>, StoreError> {
    let mut tx = pool.begin().await?;
    let Some(row) = sqlx::query(
        "SELECT thread_id, surface, selector, status FROM maidan_result_deliveries
         WHERE id = $1 FOR UPDATE",
    )
    .bind(id.0)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };
    sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = $2, last_error = $3, updated_at = now(){attempts}
         WHERE id = $1",
        attempts = if count_attempt {
            ", attempts = attempts + 1"
        } else {
            ""
        }
    ))
    .bind(id.0)
    .bind(to)
    .bind(why)
    .execute(&mut *tx)
    .await?;
    let event = if row.get::<String, _>("status") == to {
        None
    } else {
        let thread_id = ThreadId(row.get("thread_id"));
        let (workspace_id, channel_id) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
        Some(
            events::append_in_tx(
                &mut tx,
                &Event::ResultDeliveryFailed {
                    occurred_at: Utc::now(),
                    workspace_id,
                    channel_id,
                    thread_id,
                    surface: row.get("surface"),
                    selector: row.get("selector"),
                    status: to.to_string(),
                    last_error: Some(why.to_string()),
                },
            )
            .await?,
        )
    };
    tx.commit().await?;
    Ok(event)
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

pub async fn get_by_id(
    pool: &PgPool,
    thread_id: ThreadId,
    id: ResultDeliveryId,
) -> Result<Option<ResultDelivery>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_result_deliveries
         WHERE id = $1 AND thread_id = $2"
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
    pool: &PgPool,
    thread_id: ThreadId,
    id: ResultDeliveryId,
) -> Result<Option<ResultDelivery>, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_result_deliveries
         SET status = '{pending}', last_error = NULL, updated_at = now()
         WHERE id = $1 AND thread_id = $2
         RETURNING {COLS}",
        pending = status::PENDING
    ))
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
    pool: &PgPool,
    thread_id: ThreadId,
    target: &EgressTarget,
    warnings: &[String],
) -> Result<(), StoreError> {
    let codes = (!warnings.is_empty()).then(|| warnings.join(","));
    sqlx::query(
        "UPDATE maidan_result_deliveries SET template_warnings = $4
         WHERE thread_id = $1 AND surface = $2 AND selector = $3",
    )
    .bind(thread_id.0)
    .bind(target.surface().as_str())
    .bind(target.selector())
    .bind(codes)
    .execute(pool)
    .await?;
    Ok(())
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
    pool: &sqlx::PgPool,
    workspace_id: maidan_types::WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Option<String>, StoreError> {
    let handle: Option<String> = sqlx::query_scalar(
        "SELECT rd.external_ref FROM maidan_result_deliveries rd
         JOIN maidan_threads t ON t.id = rd.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = $1
           AND rd.surface = 'github_branch' AND rd.status = 'delivered'
           AND lower(substr(rd.selector, 1, length($2) + 1)) = lower($2) || '@'
           AND rd.external_ref LIKE '%#' || $3
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
