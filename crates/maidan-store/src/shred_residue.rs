//! Find words left behind for withdrawn messages.
//!
//! A withdrawal shreds the message's key and, in the same transaction, blanks
//! or deletes every copy of its words outside the event log. This walks the
//! withdrawn messages (tombstoned rows, and every shredded key, which also
//! covers messages that arrived by federation) and reports any copy still
//! there: a non-empty message row,
//! earlier versions, an unsealed event payload, a pending webhook, egress or
//! mail copy, a search-index entry or an embedding. Under a legal hold a
//! withdrawn message keeps its earlier versions on purpose, so those are not
//! residue.

use maidan_types::WorkspaceId;
use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;

use crate::embeddings_purge::assert_registry_table;
use crate::error::StoreError;

/// One place still holding words of a withdrawn message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShredResidue {
    pub workspace_id: WorkspaceId,
    /// The message id, or the shredded content key of a federated message.
    pub subject: Uuid,
    /// The table holding the leftover copy.
    pub table: String,
}

/// Withdrawn subjects: shredded keys, and tombstoned messages, which include
/// any written before their words were sealed.
const WITHDRAWN: &str = "SELECT workspace_id, id FROM maidan_content_keys
     WHERE shredded_at IS NOT NULL
     UNION
     SELECT c.workspace_id, m.id FROM maidan_messages m
     INNER JOIN maidan_threads t ON m.thread_id = t.id
     INNER JOIN maidan_channels c ON t.channel_id = c.id
     WHERE m.tombstoned_at IS NOT NULL";
const MESSAGES: &str = "EXISTS (SELECT 1 FROM maidan_messages m WHERE m.id = k.id
     AND (m.body <> '' OR m.content IS NOT NULL OR m.metadata <> '{}'))";
const EDITS: &str = "EXISTS (SELECT 1 FROM maidan_message_edits e WHERE e.message_id = k.id)
     AND NOT EXISTS (SELECT 1 FROM maidan_preserved_messages p WHERE p.message_id = k.id)";
const WEBHOOKS: &str = "EXISTS (SELECT 1 FROM maidan_webhook_deliveries d
     INNER JOIN maidan_events ev ON ev.id = d.log_id WHERE ev.content_key_id = k.id)";
const EGRESS: &str = "EXISTS (SELECT 1 FROM maidan_egress_outbox o
     INNER JOIN maidan_events ev ON ev.id = o.source_log_id WHERE ev.content_key_id = k.id)";
const MAIL: &str = "EXISTS (SELECT 1 FROM maidan_mail_outbox o WHERE o.content_key_id = k.id)";
const PG_EVENTS: &str = "EXISTS (SELECT 1 FROM maidan_events ev WHERE ev.content_key_id = k.id
     AND (NOT jsonb_exists(ev.payload, 'sealed')
          OR coalesce(ev.payload->'message'->>'body', '') <> ''
          OR jsonb_exists(ev.payload->'message', 'metadata')
          OR jsonb_exists(ev.payload->'message', 'content')))";
const SQLITE_EVENTS: &str = "EXISTS (SELECT 1 FROM maidan_events ev WHERE ev.content_key_id = k.id
     AND (json_type(ev.payload, '$.sealed') IS NULL
          OR coalesce(json_extract(ev.payload, '$.message.body'), '') <> ''
          OR json_type(ev.payload, '$.message.metadata') IS NOT NULL
          OR json_type(ev.payload, '$.message.content') IS NOT NULL))";
const SQLITE_SEARCH: &str =
    "EXISTS (SELECT 1 FROM maidan_messages_fts_map f WHERE f.message_id = k.id)";

fn embedding_check(table: &str) -> Result<String, StoreError> {
    assert_registry_table(table)?;
    Ok(format!(
        "EXISTS (SELECT 1 FROM {table} x WHERE x.message_id = k.id)"
    ))
}

/// Every leftover copy of withdrawn words, in `workspace_id` or everywhere.
pub async fn find_shred_residue_postgres(
    pool: &PgPool,
    workspace_id: Option<WorkspaceId>,
) -> Result<Vec<ShredResidue>, StoreError> {
    let mut checks: Vec<(String, String)> = vec![
        ("maidan_messages".into(), MESSAGES.into()),
        ("maidan_message_edits".into(), EDITS.into()),
        ("maidan_events".into(), PG_EVENTS.into()),
        ("maidan_webhook_deliveries".into(), WEBHOOKS.into()),
        ("maidan_egress_outbox".into(), EGRESS.into()),
        ("maidan_mail_outbox".into(), MAIL.into()),
    ];
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(pool)
        .await?;
    for table in tables {
        let check = embedding_check(&table)?;
        checks.push((table, check));
    }
    let mut found = Vec::new();
    for (table, check) in checks {
        let sql = format!(
            "SELECT k.workspace_id, k.id FROM ({WITHDRAWN}) k
             WHERE ($1::uuid IS NULL OR k.workspace_id = $1) AND {check}
             ORDER BY k.workspace_id, k.id"
        );
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(&sql)
            .bind(workspace_id.map(|w| w.0))
            .fetch_all(pool)
            .await?;
        found.extend(rows.into_iter().map(|(ws, subject)| ShredResidue {
            workspace_id: WorkspaceId(ws),
            subject,
            table: table.clone(),
        }));
    }
    Ok(found)
}

/// Every leftover copy of withdrawn words, in `workspace_id` or everywhere.
pub async fn find_shred_residue_sqlite(
    pool: &SqlitePool,
    workspace_id: Option<WorkspaceId>,
) -> Result<Vec<ShredResidue>, StoreError> {
    let mut checks: Vec<(String, String)> = vec![
        ("maidan_messages".into(), MESSAGES.into()),
        ("maidan_message_edits".into(), EDITS.into()),
        ("maidan_events".into(), SQLITE_EVENTS.into()),
        ("maidan_webhook_deliveries".into(), WEBHOOKS.into()),
        ("maidan_egress_outbox".into(), EGRESS.into()),
        ("maidan_mail_outbox".into(), MAIL.into()),
        ("maidan_messages_fts".into(), SQLITE_SEARCH.into()),
    ];
    let tables: Vec<String> = sqlx::query_scalar("SELECT table_name FROM maidan_embedding_models")
        .fetch_all(pool)
        .await?;
    for table in tables {
        let check = embedding_check(&table)?;
        checks.push((table, check));
    }
    let mut found = Vec::new();
    for (table, check) in checks {
        let sql = format!(
            "SELECT k.workspace_id, k.id FROM ({WITHDRAWN}) k
             WHERE (?1 IS NULL OR k.workspace_id = ?1) AND {check}
             ORDER BY k.workspace_id, k.id"
        );
        let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(&sql)
            .bind(workspace_id.map(|w| w.0))
            .fetch_all(pool)
            .await?;
        found.extend(rows.into_iter().map(|(ws, subject)| ShredResidue {
            workspace_id: WorkspaceId(ws),
            subject,
            table: table.clone(),
        }));
    }
    Ok(found)
}
