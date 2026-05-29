use chrono::{DateTime, Utc};
use maidan_types::{
    FsmHook, FsmHookId, FsmHookWithSecret, NewFsmHook, SlashHandlerKind, ThreadState, WorkspaceId,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

const COLS: &str =
    "id, workspace_id, label, from_state, to_state, handler_kind, handler_target, secret_ciphertext, enabled, created_at, revoked_at";

pub async fn create(pool: &PgPool, new: NewFsmHook) -> Result<FsmHook, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_fsm_hooks
            (id, workspace_id, label, from_state, to_state, handler_kind, handler_target, secret_ciphertext)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING {COLS}"
    ))
    .bind(id)
    .bind(new.workspace_id.0)
    .bind(&new.label)
    .bind(state_to_opt_str(new.from_state))
    .bind(state_to_opt_str(new.to_state))
    .bind(new.handler_kind.as_str())
    .bind(&new.handler_target)
    .bind(&new.secret_ciphertext)
    .fetch_one(pool)
    .await?;
    row_to_hook(&row)
}

pub async fn get(pool: &PgPool, id: FsmHookId) -> Result<FsmHookWithSecret, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_fsm_hooks WHERE id = $1"
    ))
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

pub async fn list(pool: &PgPool, workspace_id: WorkspaceId) -> Result<Vec<FsmHook>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_fsm_hooks
         WHERE workspace_id = $1 AND revoked_at IS NULL
         ORDER BY created_at ASC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_hook).collect()
}

pub async fn list_matching(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    from_state: ThreadState,
    to_state: ThreadState,
) -> Result<Vec<FsmHookWithSecret>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS}
         FROM maidan_fsm_hooks
         WHERE workspace_id = $1
           AND enabled = TRUE
           AND revoked_at IS NULL
           AND (from_state IS NULL OR from_state = $2)
           AND (to_state IS NULL OR to_state = $3)"
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

pub async fn revoke(pool: &PgPool, id: FsmHookId) -> Result<FsmHook, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_fsm_hooks
         SET enabled = FALSE, revoked_at = NOW()
         WHERE id = $1 AND revoked_at IS NULL
         RETURNING {COLS}"
    ))
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_hook(&row)
}

fn state_to_opt_str(state: Option<ThreadState>) -> Option<&'static str> {
    state.map(|s| s.as_str())
}

fn row_to_hook(row: &sqlx::postgres::PgRow) -> Result<FsmHook, StoreError> {
    let handler_kind: String = row.get("handler_kind");
    Ok(FsmHook {
        id: FsmHookId(row.get("id")),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        label: row.get("label"),
        from_state: parse_opt_state(row.get::<Option<String>, _>("from_state"))?,
        to_state: parse_opt_state(row.get::<Option<String>, _>("to_state"))?,
        handler_kind: SlashHandlerKind::parse(&handler_kind)
            .ok_or_else(|| StoreError::InvalidInput(format!("bad handler_kind: {handler_kind}")))?,
        handler_target: row.get("handler_target"),
        enabled: row.get("enabled"),
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
