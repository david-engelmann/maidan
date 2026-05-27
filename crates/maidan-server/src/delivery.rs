//! Delivery cursor helpers for subscribe and federation paths.

use maidan_store::Store;
use maidan_types::WorkspaceId;

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
) -> Result<i64, maidan_store::StoreError> {
    let (Some(consumer_id), Some(workspace_id)) = (consumer_id, workspace_id) else {
        return Ok(requested_after_id);
    };
    let cursor = store.get_delivery_cursor(consumer_id, workspace_id).await?;
    Ok(requested_after_id.max(cursor))
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
