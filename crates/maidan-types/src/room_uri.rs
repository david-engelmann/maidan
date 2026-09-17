//! Stable `maidan://` room URIs.
//!
//! Hierarchical address for a workspace (room) and the channel / thread /
//! message path under it:
//!
//! ```text
//! maidan://{workspace_id}/channels/{channel_id}/threads/{thread_id}/messages/{message_id}[#sha256:<hex>]
//! ```
//!
//! **The authority is always the workspace UUID.** A renameable handle is
//! an alias advertised on the room card and `/.well-known/maidan-room`; it is
//! never stored in the URI. A handle rename therefore cannot break a stored id.
//!
//! This is **not** the `maidan:event/{id}` strong-ref pin, and
//! **not** the MCP resource forms `maidan://threads/{id}` /
//! `maidan://workspaces/{id}`. Those stay. A [`RoomUri`] parse rejects them (no
//! UUID authority, or a non-hierarchical path).
//!
//! The optional fragment is a `sha256:<hex>` content hash.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::event_chain::is_well_formed_hash;
use crate::ids::{ChannelId, MessageId, ThreadId, WorkspaceId};

/// URI scheme. Breaking authority or path grammar is a new scheme, not
/// a silent reshape of `maidan`.
pub const ROOM_URI_SCHEME: &str = "maidan";

/// Path template published on the well-known discovery document.
pub const ROOM_URI_TEMPLATE: &str =
    "maidan://{workspace_id}/channels/{channel_id}/threads/{thread_id}/messages/{message_id}";

/// A hierarchical room address. `workspace_id` is required; each deeper
/// segment requires its parent (a message implies thread and channel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RoomUri {
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<ChannelId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    /// Optional content hash (`sha256:<hex>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RoomUriError {
    #[error("room URI must use the maidan scheme")]
    WrongScheme,
    #[error("room URI authority must be a workspace UUID, not a handle")]
    AuthorityNotUuid,
    #[error("room URI path is not a channels/threads/messages hierarchy")]
    MalformedPath,
    #[error("room URI content_hash fragment must be sha256:<hex>")]
    MalformedHash,
    #[error("room URI must not carry a query string")]
    QueryNotAllowed,
}

impl RoomUri {
    pub fn workspace(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id,
            channel_id: None,
            thread_id: None,
            message_id: None,
            content_hash: None,
        }
    }

    pub fn channel(workspace_id: WorkspaceId, channel_id: ChannelId) -> Self {
        Self {
            workspace_id,
            channel_id: Some(channel_id),
            thread_id: None,
            message_id: None,
            content_hash: None,
        }
    }

    pub fn thread(workspace_id: WorkspaceId, channel_id: ChannelId, thread_id: ThreadId) -> Self {
        Self {
            workspace_id,
            channel_id: Some(channel_id),
            thread_id: Some(thread_id),
            message_id: None,
            content_hash: None,
        }
    }

    pub fn message(
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        message_id: MessageId,
    ) -> Self {
        Self {
            workspace_id,
            channel_id: Some(channel_id),
            thread_id: Some(thread_id),
            message_id: Some(message_id),
            content_hash: None,
        }
    }

    pub fn with_content_hash(mut self, hash: impl Into<String>) -> Result<Self, RoomUriError> {
        let hash = hash.into();
        if !is_well_formed_hash(&hash) {
            return Err(RoomUriError::MalformedHash);
        }
        self.content_hash = Some(hash);
        Ok(self)
    }

    /// Parse a hierarchical room URI. Handle-shaped hosts fail — stored
    /// ids are UUIDs.
    pub fn parse(input: &str) -> Result<Self, RoomUriError> {
        let (base, fragment) = match input.split_once('#') {
            Some((base, frag)) => (base, Some(frag)),
            None => (input, None),
        };
        if base.contains('?') {
            return Err(RoomUriError::QueryNotAllowed);
        }
        let rest = base
            .strip_prefix("maidan://")
            .ok_or(RoomUriError::WrongScheme)?;
        let (host, path) = match rest.split_once('/') {
            Some((host, path)) => (host, path),
            None => (rest, ""),
        };
        if host.is_empty() || host.contains('@') || host.contains(':') {
            return Err(RoomUriError::AuthorityNotUuid);
        }
        let workspace_id = parse_host(host).map(WorkspaceId)?;
        let content_hash = match fragment {
            Some("") => None,
            Some(frag) => {
                if !is_well_formed_hash(frag) {
                    return Err(RoomUriError::MalformedHash);
                }
                Some(frag.to_string())
            }
            None => None,
        };

        if path.is_empty() {
            return Ok(Self {
                workspace_id,
                channel_id: None,
                thread_id: None,
                message_id: None,
                content_hash,
            });
        }

        let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        match segs.as_slice() {
            ["channels", ch] => Ok(Self {
                workspace_id,
                channel_id: Some(ChannelId(parse_path_id(ch)?)),
                thread_id: None,
                message_id: None,
                content_hash,
            }),
            ["channels", ch, "threads", th] => Ok(Self {
                workspace_id,
                channel_id: Some(ChannelId(parse_path_id(ch)?)),
                thread_id: Some(ThreadId(parse_path_id(th)?)),
                message_id: None,
                content_hash,
            }),
            ["channels", ch, "threads", th, "messages", mid] => Ok(Self {
                workspace_id,
                channel_id: Some(ChannelId(parse_path_id(ch)?)),
                thread_id: Some(ThreadId(parse_path_id(th)?)),
                message_id: Some(MessageId(parse_path_id(mid)?)),
                content_hash,
            }),
            _ => Err(RoomUriError::MalformedPath),
        }
    }
}

fn parse_host(s: &str) -> Result<Uuid, RoomUriError> {
    Uuid::parse_str(s).map_err(|_| RoomUriError::AuthorityNotUuid)
}

fn parse_path_id(s: &str) -> Result<Uuid, RoomUriError> {
    Uuid::parse_str(s).map_err(|_| RoomUriError::MalformedPath)
}

impl fmt::Display for RoomUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "maidan://{}", self.workspace_id.0)?;
        if let Some(ch) = self.channel_id {
            write!(f, "/channels/{}", ch.0)?;
            if let Some(th) = self.thread_id {
                write!(f, "/threads/{}", th.0)?;
                if let Some(mid) = self.message_id {
                    write!(f, "/messages/{}", mid.0)?;
                }
            }
        }
        if let Some(hash) = &self.content_hash {
            write!(f, "#{hash}")?;
        }
        Ok(())
    }
}

impl FromStr for RoomUri {
    type Err = RoomUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_chain::HASH_PREFIX;

    fn ws() -> WorkspaceId {
        WorkspaceId(Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap())
    }
    fn ch() -> ChannelId {
        ChannelId(Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap())
    }
    fn th() -> ThreadId {
        ThreadId(Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap())
    }
    fn mid() -> MessageId {
        MessageId(Uuid::parse_str("44444444-4444-4444-8444-444444444444").unwrap())
    }

    #[test]
    fn workspace_uri_round_trips() {
        let uri = RoomUri::workspace(ws());
        let s = uri.to_string();
        assert_eq!(s, "maidan://11111111-1111-4111-8111-111111111111");
        assert_eq!(RoomUri::parse(&s).unwrap(), uri);
    }

    #[test]
    fn hierarchical_uri_round_trips() {
        let uri = RoomUri::message(ws(), ch(), th(), mid());
        let s = uri.to_string();
        assert_eq!(
            s,
            "maidan://11111111-1111-4111-8111-111111111111/channels/22222222-2222-4222-8222-222222222222/threads/33333333-3333-4333-8333-333333333333/messages/44444444-4444-4444-8444-444444444444"
        );
        assert_eq!(RoomUri::parse(&s).unwrap(), uri);
        assert_eq!(
            RoomUri::parse(&RoomUri::channel(ws(), ch()).to_string()).unwrap(),
            RoomUri::channel(ws(), ch())
        );
        assert_eq!(
            RoomUri::parse(&RoomUri::thread(ws(), ch(), th()).to_string()).unwrap(),
            RoomUri::thread(ws(), ch(), th())
        );
    }

    #[test]
    fn content_hash_fragment_round_trips() {
        let hash = format!("{HASH_PREFIX}{}", "ab".repeat(32));
        let uri = RoomUri::message(ws(), ch(), th(), mid())
            .with_content_hash(hash.clone())
            .unwrap();
        let parsed = RoomUri::parse(&uri.to_string()).unwrap();
        assert_eq!(parsed.content_hash.as_deref(), Some(hash.as_str()));
    }

    #[test]
    fn handle_shaped_authority_is_rejected() {
        let err = RoomUri::parse("maidan://acme/channels/22222222-2222-4222-8222-222222222222")
            .expect_err("handle host");
        assert_eq!(err, RoomUriError::AuthorityNotUuid);
    }

    #[test]
    fn mcp_resource_forms_are_not_room_uris() {
        assert_eq!(
            RoomUri::parse("maidan://threads/33333333-3333-4333-8333-333333333333")
                .expect_err("mcp thread"),
            RoomUriError::AuthorityNotUuid
        );
        assert_eq!(
            RoomUri::parse("maidan://workspaces/11111111-1111-4111-8111-111111111111")
                .expect_err("mcp workspace"),
            RoomUriError::AuthorityNotUuid
        );
        assert_eq!(
            RoomUri::parse("maidan:event/42").expect_err("strong ref"),
            RoomUriError::WrongScheme
        );
        assert_eq!(
            RoomUri::parse("maidan:message/44444444-4444-4444-8444-444444444444")
                .expect_err("strong ref message"),
            RoomUriError::WrongScheme
        );
    }

    #[test]
    fn malformed_path_and_hash_fail_closed() {
        let ws = "maidan://11111111-1111-4111-8111-111111111111";
        assert_eq!(
            RoomUri::parse(&format!(
                "{ws}/threads/33333333-3333-4333-8333-333333333333"
            ))
            .expect_err("thread without channel"),
            RoomUriError::MalformedPath
        );
        assert_eq!(
            RoomUri::parse(&format!("{ws}?foo=1")).expect_err("query"),
            RoomUriError::QueryNotAllowed
        );
        assert_eq!(
            RoomUri::parse(&format!("{ws}#not-a-hash")).expect_err("hash"),
            RoomUriError::MalformedHash
        );
        assert_eq!(
            RoomUri::parse(&format!("{ws}/channels/not-a-uuid")).expect_err("path id"),
            RoomUriError::MalformedPath
        );
        assert!(RoomUri::workspace(WorkspaceId::new())
            .with_content_hash("deadbeef")
            .is_err());
    }
}
