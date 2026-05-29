use maidan_types::{ApiTokenId, TokenQuota};
use sqlx::SqlitePool;

use crate::error::StoreError;

pub async fn replace(
    pool: &SqlitePool,
    token_id: ApiTokenId,
    quotas: &[TokenQuota],
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM maidan_token_quotas WHERE token_id = ?")
        .bind(token_id.0)
        .execute(&mut *tx)
        .await?;
    for q in quotas {
        sqlx::query(
            "INSERT INTO maidan_token_quotas (token_id, capability, max_per_window, window_secs)
             VALUES (?, ?, ?, ?)",
        )
        .bind(token_id.0)
        .bind(&q.capability)
        .bind(i64::from(q.max_per_window))
        .bind(
            i64::try_from(q.window_secs)
                .map_err(|_| StoreError::InvalidInput("window_secs out of range".into()))?,
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool, token_id: ApiTokenId) -> Result<Vec<TokenQuota>, StoreError> {
    let rows = sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT capability, max_per_window, window_secs
         FROM maidan_token_quotas
         WHERE token_id = ?
         ORDER BY capability",
    )
    .bind(token_id.0)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_quota).collect()
}

fn row_to_quota((capability, max, window): (String, i64, i64)) -> Result<TokenQuota, StoreError> {
    if max <= 0 || window <= 0 {
        return Err(StoreError::InvalidInput(
            "invalid quota row in database".into(),
        ));
    }
    Ok(TokenQuota {
        capability,
        max_per_window: u32::try_from(max)
            .map_err(|_| StoreError::InvalidInput("max_per_window out of range".into()))?,
        window_secs: u64::try_from(window)
            .map_err(|_| StoreError::InvalidInput("window_secs out of range".into()))?,
    })
}
