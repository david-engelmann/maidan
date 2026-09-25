//! Delivery cursor helpers for subscribe and federation paths.

use maidan_store::{Store, StoreError};
use maidan_types::{ProjectorShape, StoredEvent, WorkspaceId};

const MAX_CONSUMER_ID_LEN: usize = 256;

pub fn validate_consumer_id(consumer_id: &str) -> Result<(), String> {
    if consumer_id.is_empty() {
        return Err("consumer_id must not be empty".into());
    }
    if consumer_id.len() > MAX_CONSUMER_ID_LEN {
        return Err("consumer_id too long".into());
    }
    if !consumer_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
    {
        return Err("consumer_id may only contain ASCII letters, digits, and : - _ .".into());
    }
    Ok(())
}

/// `max(requested_after_id, persisted cursor)` when both `consumer_id` and `workspace_id` are set.
pub async fn effective_subscribe_after_id(
    store: &dyn Store,
    consumer_id: Option<&str>,
    workspace_id: Option<WorkspaceId>,
    requested_after_id: i64,
) -> Result<i64, StoreError> {
    let (Some(consumer_id), Some(workspace_id)) = (consumer_id, workspace_id) else {
        return Ok(requested_after_id);
    };
    let cursor = store.get_delivery_cursor(consumer_id, workspace_id).await?;
    Ok(requested_after_id.max(cursor))
}

/// Fail loud when a subscribe / backfill cursor points into a pruned gap.
/// No workspace (live-only, no replay) is never too old.
pub async fn ensure_subscribe_cursor(
    store: &dyn Store,
    workspace_id: Option<WorkspaceId>,
    after_id: i64,
) -> Result<(), StoreError> {
    let Some(workspace_id) = workspace_id else {
        return Ok(());
    };
    store.ensure_cursor_fresh(workspace_id, after_id).await
}

/// HTTP backfill for a projector shape: freshness check, then a page of matching
/// rows. Unfiltered shapes use a single `list_events_after`; filtered shapes
/// keep paging until `limit` matches or the log ends (never under-fill by
/// dropping non-matching rows from a single page).
/// With `visible`, only the events that caller may read are returned; paging
/// continues past hidden rows, so a page is never short while more readable
/// events remain.
pub async fn list_events_for_shape(
    store: &dyn Store,
    shape: &ProjectorShape,
    after_id: i64,
    limit: i64,
    mut visible: Option<&mut crate::event_visibility::EventVisibility<'_>>,
) -> Result<Vec<StoredEvent>, StoreError> {
    store
        .ensure_cursor_fresh(shape.workspace_id, after_id)
        .await?;
    if visible.is_none()
        && shape.channel_id.is_none()
        && shape.thread_id.is_none()
        && shape.types.is_empty()
    {
        return store
            .list_events_after(shape.workspace_id, after_id, limit)
            .await;
    }
    let mut out = Vec::new();
    let mut after = after_id;
    let page_size = limit.max(1);
    loop {
        let page = store
            .list_events_after(shape.workspace_id, after, page_size)
            .await?;
        if page.is_empty() {
            break;
        }
        let short = (page.len() as i64) < page_size;
        for row in page {
            after = row.id;
            let mut row = row;
            let readable = match visible.as_deref_mut() {
                Some(v) => {
                    let allowed = v.allows(&row).await?;
                    if allowed {
                        v.redact_withdrawn(&mut row).await?;
                    }
                    allowed
                }
                None => true,
            };
            if readable && shape.matches_stored(&row) {
                out.push(row);
                if (out.len() as i64) >= limit {
                    return Ok(out);
                }
            }
        }
        if short {
            break;
        }
    }
    Ok(out)
}

pub fn federation_consumer_id(peer_id: maidan_types::PeerId) -> String {
    format!("federation:{}", peer_id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_consumer_id_rejects_empty_and_invalid_chars() {
        assert!(validate_consumer_id("").is_err());
        assert!(validate_consumer_id("good:agent-1").is_ok());
        assert!(validate_consumer_id("bad space").is_err());
    }
}
