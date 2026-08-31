//! Baseline mention routing: `@handle` tokens in message bodies become stored mentions.

use maidan_store::Store;
use maidan_types::{MemberId, MessageId, WorkspaceId};

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
            if end > start {
                if let Ok(s) = std::str::from_utf8(&bytes[start..end]) {
                    if !s.is_empty() {
                        handles.push(s.to_string());
                    }
                }
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
        let member = match store.get_member_by_handle(workspace_id, &handle).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if member.id == author_id {
            continue;
        }
        store.record_mention(message_id, member.id).await?;
        if !mentioned.contains(&member.id) {
            mentioned.push(member.id);
        }
    }
    Ok(mentioned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_at_handles_dedupes_and_skips_invalid() {
        assert_eq!(
            parse_at_handles("hi @alice and @bob @alice"),
            vec!["alice".to_string(), "bob".to_string()]
        );
        assert!(parse_at_handles("email user@host.com").is_empty());
        assert_eq!(parse_at_handles("@solo"), vec!["solo".to_string()]);
    }
}
