//! Web Push subscription queries (Cluster 366, N1): the `maidan_push_subscriptions`
//! table. The notification router lists a member's subscriptions to deliver a Web
//! Push message when the member has no live WebSocket.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, NewPushSubscription, PushSubscription, PushSubscriptionId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_sub(row: &sqlx::postgres::PgRow) -> PushSubscription {
    PushSubscription {
        id: PushSubscriptionId(row.get::<Uuid, _>("id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        endpoint: row.get::<String, _>("endpoint"),
        p256dh: row.get::<String, _>("p256dh"),
        auth: row.get::<String, _>("auth"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

const COLS: &str = "id, member_id, endpoint, p256dh, auth, created_at";

pub async fn add(pool: &PgPool, new: NewPushSubscription) -> Result<PushSubscription, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_push_subscriptions (id, member_id, endpoint, p256dh, auth, created_at)
         VALUES ($1, $2, $3, $4, $5, NOW())
         ON CONFLICT (member_id, endpoint) DO UPDATE SET
             p256dh = excluded.p256dh,
             auth = excluded.auth
         RETURNING {COLS}"
    ))
    .bind(id)
    .bind(new.member_id.0)
    .bind(&new.endpoint)
    .bind(&new.p256dh)
    .bind(&new.auth)
    .fetch_one(pool)
    .await?;
    Ok(row_to_sub(&row))
}

pub async fn list(pool: &PgPool, member_id: MemberId) -> Result<Vec<PushSubscription>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_push_subscriptions
         WHERE member_id = $1 ORDER BY created_at ASC, id ASC"
    ))
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_sub).collect())
}

pub async fn delete(
    pool: &PgPool,
    member_id: MemberId,
    id: PushSubscriptionId,
) -> Result<bool, StoreError> {
    let done =
        sqlx::query("DELETE FROM maidan_push_subscriptions WHERE id = $1 AND member_id = $2")
            .bind(id.0)
            .bind(member_id.0)
            .execute(pool)
            .await?;
    Ok(done.rows_affected() > 0)
}
