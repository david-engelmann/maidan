//! Optional lexical-search facets (v1.2.2).

use maidan_types::{ChannelId, MemberId, MemberKind};

/// Facets applied in addition to the workspace scope and FTS match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    pub author_id: Option<MemberId>,
    pub channel_id: Option<ChannelId>,
    /// Restrict to messages whose author has this [`MemberKind`].
    pub author_kind: Option<MemberKind>,
}

impl SearchFilters {
    pub fn is_empty(&self) -> bool {
        self.author_id.is_none() && self.channel_id.is_none() && self.author_kind.is_none()
    }
}
