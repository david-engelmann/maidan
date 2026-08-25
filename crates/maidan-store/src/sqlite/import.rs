//! Workspace import (Cluster 269) — the SQLite twin of the Postgres import. Same
//! shape (one transaction, explicit ids/state/timestamps preserved); SQLite stores
//! `metadata`/`content` as JSON TEXT and uses `?` placeholders.

use maidan_types::WorkspaceImport;
use sqlx::SqlitePool;

use crate::error::StoreError;

pub async fn import_workspace(pool: &SqlitePool, b: &WorkspaceImport) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;

    let w = &b.workspace;
    sqlx::query(
        "INSERT INTO maidan_workspaces (id, name, created_at, updated_at, tombstoned_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(w.id.0)
    .bind(&w.name)
    .bind(w.created_at)
    .bind(w.updated_at)
    .bind(w.tombstoned_at)
    .execute(&mut *tx)
    .await?;

    for m in &b.members {
        sqlx::query(
            "INSERT INTO maidan_members
                (id, workspace_id, handle, display_name, kind, created_at, updated_at, tombstoned_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(m.id.0)
        .bind(m.workspace_id.0)
        .bind(&m.handle)
        .bind(m.display_name.as_deref())
        .bind(m.kind.as_str())
        .bind(m.created_at)
        .bind(m.updated_at)
        .bind(m.tombstoned_at)
        .execute(&mut *tx)
        .await?;
    }

    for c in &b.channels {
        sqlx::query(
            "INSERT INTO maidan_channels
                (id, workspace_id, name, topic, private, created_at, updated_at, tombstoned_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(c.id.0)
        .bind(c.workspace_id.0)
        .bind(&c.name)
        .bind(c.topic.as_deref())
        .bind(c.private)
        .bind(c.created_at)
        .bind(c.updated_at)
        .bind(c.tombstoned_at)
        .execute(&mut *tx)
        .await?;
    }

    for cm in &b.channel_members {
        sqlx::query(
            "INSERT INTO maidan_channel_members (channel_id, member_id, role, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(cm.channel_id.0)
        .bind(cm.member_id.0)
        .bind(cm.role.as_str())
        .bind(cm.created_at)
        .execute(&mut *tx)
        .await?;
    }

    for t in &b.threads {
        sqlx::query(
            "INSERT INTO maidan_threads
                (id, channel_id, parent_thread_id, title, state, assignee_id,
                 assignment_expires_at, created_at, updated_at, tombstoned_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(t.id.0)
        .bind(t.channel_id.0)
        .bind(t.parent_thread_id.map(|p| p.0))
        .bind(t.title.as_deref())
        .bind(t.state.as_str())
        .bind(t.assignee_id.map(|a| a.0))
        .bind(t.assignment_expires_at)
        .bind(t.created_at)
        .bind(t.updated_at)
        .bind(t.tombstoned_at)
        .execute(&mut *tx)
        .await?;
    }

    for msg in &b.messages {
        let metadata_text = serde_json::to_string(&msg.metadata)?;
        let content_text = msg
            .content
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        sqlx::query(
            "INSERT INTO maidan_messages
                (id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(msg.id.0)
        .bind(msg.thread_id.0)
        .bind(msg.author_id.0)
        .bind(&msg.body)
        .bind(&metadata_text)
        .bind(&content_text)
        .bind(msg.posted_at)
        .bind(msg.edited_at)
        .bind(msg.tombstoned_at)
        .execute(&mut *tx)
        .await?;
    }

    for e in &b.message_edits {
        sqlx::query(
            "INSERT INTO maidan_message_edits (message_id, editor_id, body_before, body_after, edited_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(e.message_id.0)
        .bind(e.editor_id.0)
        .bind(&e.body_before)
        .bind(&e.body_after)
        .bind(e.edited_at)
        .execute(&mut *tx)
        .await?;
    }

    for p in &b.pins {
        sqlx::query(
            "INSERT INTO maidan_pins (thread_id, message_id, member_id, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(p.thread_id.0)
        .bind(p.message_id.0)
        .bind(p.member_id.0)
        .bind(p.created_at)
        .execute(&mut *tx)
        .await?;
    }

    for r in &b.references {
        sqlx::query(
            "INSERT INTO maidan_references (id, src_kind, src_id, dst_kind, dst_id, relation, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(r.id)
        .bind(r.src_kind.as_str())
        .bind(r.src_id)
        .bind(r.dst_kind.as_str())
        .bind(r.dst_id)
        .bind(&r.relation)
        .bind(r.created_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
