//! Baseline mention routing: `@handle` tokens in message bodies become stored mentions.

use maidan_store::{Store, StoreError};
use maidan_types::{Member, MemberId, MessageId, WorkspaceId};

use crate::RouterError;

/// Extract unique `@handle` tokens from `body` (ASCII handles: letter/digit/`_`/`-`).
pub fn parse_at_handles(body: &str) -> Vec<String> {
    let mut handles = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' && i + 1 < bytes.len() {
            if i > 0 && is_handle_char(bytes[i - 1]) {
                i += 1;
                continue;
            }
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && is_handle_char(bytes[end]) {
                end += 1;
            }
            if end > start
                && let Ok(s) = std::str::from_utf8(&bytes[start..end])
                && !s.is_empty()
            {
                handles.push(s.to_string());
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    handles.sort();
    handles.dedup();
    handles
}

fn is_handle_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Resolve `@handle` mentions in `body` and persist them for `message_id`.
///
/// Unknown handles are skipped. The author is not mentioned when they @ themselves.
pub async fn route_mentions_in_message(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    message_id: MessageId,
    author_id: MemberId,
    body: &str,
) -> Result<Vec<MemberId>, RouterError> {
    let mut mentioned = Vec::new();
    for handle in parse_at_handles(body) {
        let Some(member) = member_for_handle(store, workspace_id, &handle).await? else {
            continue;
        };
        // The store's own lookup hides a tombstone. A store that still
        // returns one must not record it.
        if member.tombstoned_at.is_some() || member.id == author_id {
            continue;
        }
        if mentioned.contains(&member.id) {
            continue;
        }
        store.record_mention(message_id, member.id).await?;
        mentioned.push(member.id);
    }
    Ok(mentioned)
}

/// The member `handle` names. `None` when there is no such member. A real
/// store error is returned. An uppercase handle is tried again in lowercase,
/// so `@Alice` reaches a member stored as `alice` even when the store compares
/// handles as written.
async fn member_for_handle(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    handle: &str,
) -> Result<Option<Member>, RouterError> {
    match store.get_member_by_handle(workspace_id, handle).await {
        Ok(member) => Ok(Some(member)),
        Err(StoreError::NotFound) if handle.bytes().any(|b| b.is_ascii_uppercase()) => {
            match store
                .get_member_by_handle(workspace_id, &handle.to_ascii_lowercase())
                .await
            {
                Ok(member) => Ok(Some(member)),
                Err(StoreError::NotFound) => Ok(None),
                Err(err) => Err(err.into()),
            }
        }
        Err(StoreError::NotFound) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use maidan_store::prelude::*;

    use super::*;

    #[test]
    fn parse_at_handles_dedupes_and_skips_invalid() {
        assert_eq!(
            parse_at_handles("hi @alice and @bob @alice"),
            vec!["alice".to_string(), "bob".to_string()]
        );
        assert!(parse_at_handles("email user@host.com").is_empty());
        assert!(parse_at_handles("first.last@example.com").is_empty());
        assert_eq!(parse_at_handles("@solo"), vec!["solo".to_string()]);
        assert_eq!(parse_at_handles("(@alice)"), vec!["alice".to_string()]);
        assert_eq!(parse_at_handles("\"@alice\""), vec!["alice".to_string()]);
        assert_eq!(parse_at_handles(",@alice"), vec!["alice".to_string()]);
        assert_eq!(parse_at_handles(". @alice"), vec!["alice".to_string()]);
        assert_eq!(parse_at_handles("hi/@alice"), vec!["alice".to_string()]);
    }

    async fn workspace() -> (
        maidan_store::SqliteStore,
        maidan_types::WorkspaceId,
        maidan_types::Member,
        maidan_types::Member,
        maidan_types::MessageId,
        sqlx::SqlitePool,
    ) {
        use maidan_store::{prelude::*, run_sqlite_migrations, SqliteStore};
        use maidan_types::*;

        let pool = maidan_search::sqlite_pool_options()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store = SqliteStore::for_tests(pool.clone());
        let ws = store
            .create_workspace(NewWorkspace {
                name: "mentions".into(),
            })
            .await
            .unwrap();
        let alice = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "alice".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let bob = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "bob".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                title: None,
                description: None,
                parent_thread_id: None,
                created_by: None,
            })
            .await
            .unwrap();
        let msg = store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: bob.id,
                body: "hey".into(),
                metadata: Default::default(),
                content: None,
            })
            .await
            .unwrap();
        (store, ws.id, alice, bob, msg.id, pool)
    }

    #[tokio::test]
    async fn alice_in_either_case_is_mentioned_once_and_a_tombstone_is_skipped() {
        let (store, ws, alice, bob, message_id, pool) = workspace().await;
        let mentioned =
            route_mentions_in_message(&store, ws, message_id, bob.id, "hey @Alice and @alice")
                .await
                .unwrap();
        assert_eq!(mentioned, vec![alice.id]);
        let rows = store.list_mentions_for_member(alice.id, 10).await.unwrap();
        assert_eq!(rows.len(), 1);

        sqlx::query("UPDATE maidan_members SET tombstoned_at = '2026-01-01 00:00:00' WHERE id = ?")
            .bind(alice.id.0)
            .execute(&pool)
            .await
            .unwrap();
        let mentioned = route_mentions_in_message(&store, ws, message_id, bob.id, "@Alice")
            .await
            .unwrap();
        assert!(mentioned.is_empty());
    }

    #[tokio::test]
    async fn a_store_failure_other_than_not_found_is_returned() {
        let (store, ws, _alice, bob, message_id, pool) = workspace().await;
        sqlx::query("ALTER TABLE maidan_members RENAME TO maidan_members_gone")
            .execute(&pool)
            .await
            .unwrap();
        let err = route_mentions_in_message(&store, ws, message_id, bob.id, "@alice")
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                crate::RouterError::Store(maidan_store::StoreError::Database(_))
            ),
            "{err:?}"
        );
    }
}
