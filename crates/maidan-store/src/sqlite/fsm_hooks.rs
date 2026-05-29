use chrono::{DateTime, Utc};
use maidan_types::{
    FsmHook, FsmHookId, FsmHookWithSecret, NewFsmHook, SlashHandlerKind, ThreadState, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str =
    "id, workspace_id, label, from_state, to_state, handler_kind, handler_target, secret_ciphertext, enabled, created_at, revoked_at";

pub async fn create(pool: &SqlitePool, new: NewFsmHook) -> Result<FsmHook, StoreError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO maidan_fsm_hooks
            (id, workspace_id, label, from_state, to_state, handler_kind, handler_target, secret_ciphertext)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.label)
    .bind(state_to_opt_str(new.from_state))
    .bind(state_to_opt_str(new.to_state))
    .bind(new.handler_kind.as_str())
    .bind(&new.handler_target)
    .bind(&new.secret_ciphertext)
    .execute(pool)
    .await?;
    get(pool, FsmHookId(id)).await.map(|row| row.hook)
}

pub async fn get(pool: &SqlitePool, id: FsmHookId) -> Result<FsmHookWithSecret, StoreError> {
    let row = sqlx::query(&format!("SELECT {COLS} FROM maidan_fsm_hooks WHERE id = ?"))
        .bind(id.0)
        .fetch_optional(pool)
        .await?
        .ok_or(StoreError::NotFound)?;
    let hook = row_to_hook(&row)?;
    Ok(FsmHookWithSecret {
        secret_ciphertext: row.get("secret_ciphertext"),
        hook,
    })
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<FsmHook>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_fsm_hooks
         WHERE workspace_id = ? AND revoked_at IS NULL
         ORDER BY created_at ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_hook).collect()
}

pub async fn list_matching(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    from_state: ThreadState,
    to_state: ThreadState,
) -> Result<Vec<FsmHookWithSecret>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_fsm_hooks
         WHERE workspace_id = ?
           AND enabled = 1
           AND revoked_at IS NULL
           AND (from_state IS NULL OR from_state = ?)
           AND (to_state IS NULL OR to_state = ?)"
    ))
    .bind(workspace_id.0)
    .bind(from_state.as_str())
    .bind(to_state.as_str())
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            let hook = row_to_hook(row)?;
            Ok(FsmHookWithSecret {
                secret_ciphertext: row.get("secret_ciphertext"),
                hook,
            })
        })
        .collect()
}

pub async fn revoke(pool: &SqlitePool, id: FsmHookId) -> Result<FsmHook, StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_fsm_hooks
         SET enabled = 0, revoked_at = CURRENT_TIMESTAMP
         WHERE id = ? AND revoked_at IS NULL",
    )
    .bind(id.0)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    get(pool, id).await.map(|row| row.hook)
}

fn state_to_opt_str(state: Option<ThreadState>) -> Option<&'static str> {
    state.map(|s| s.as_str())
}

fn row_to_hook(row: &sqlx::sqlite::SqliteRow) -> Result<FsmHook, StoreError> {
    let handler_kind: String = row.get("handler_kind");
    let enabled: i64 = row.get("enabled");
    Ok(FsmHook {
        id: FsmHookId(row.get::<Uuid, _>("id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        label: row.get("label"),
        from_state: parse_opt_state(row.get::<Option<String>, _>("from_state"))?,
        to_state: parse_opt_state(row.get::<Option<String>, _>("to_state"))?,
        handler_kind: SlashHandlerKind::parse(&handler_kind)
            .ok_or_else(|| StoreError::InvalidInput(format!("bad handler_kind: {handler_kind}")))?,
        handler_target: row.get("handler_target"),
        enabled: enabled != 0,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        revoked_at: row.get("revoked_at"),
    })
}

fn parse_opt_state(raw: Option<String>) -> Result<Option<ThreadState>, StoreError> {
    match raw {
        None => Ok(None),
        Some(s) => ThreadState::parse(&s)
            .ok_or_else(|| StoreError::InvalidInput(format!("bad thread state: {s}")))
            .map(Some),
    }
}
