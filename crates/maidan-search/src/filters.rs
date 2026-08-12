//! Optional lexical-search facets (v1.2.2).

use maidan_types::{ChannelId, MemberId, MemberKind};

/// Facets applied in addition to the workspace scope and FTS match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    pub author_id: Option<MemberId>,
    pub channel_id: Option<ChannelId>,
    /// Restrict to messages whose author has this [`MemberKind`].
    pub author_kind: Option<MemberKind>,
    /// RBAC pre-filter (Cluster 200): channels whose messages must be excluded at
    /// the query level — the private channels the caller isn't a member of. Not a
    /// user-settable facet; the server computes it so inaccessible hits never
    /// crowd out the requested `limit` (the thread-level post-filter stays the
    /// authoritative, DM-aware check). Empty = no restriction.
    pub deny_channels: Vec<ChannelId>,
}

impl SearchFilters {
    /// `true` when no **user** facet is set. The RBAC `deny_channels` pre-filter is
    /// deliberately excluded — it is applied by the query regardless.
    pub fn is_empty(&self) -> bool {
        self.author_id.is_none() && self.channel_id.is_none() && self.author_kind.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::{ChannelId, MemberId, MemberKind};

    #[test]
    fn is_empty_when_no_facets_set() {
        assert!(SearchFilters::default().is_empty());
    }

    #[test]
    fn is_not_empty_when_any_facet_set() {
        let with_author = SearchFilters {
            author_id: Some(MemberId(uuid::Uuid::new_v4())),
            ..Default::default()
        };
        assert!(!with_author.is_empty());

        let with_channel = SearchFilters {
            channel_id: Some(ChannelId(uuid::Uuid::new_v4())),
            ..Default::default()
        };
        assert!(!with_channel.is_empty());

        let with_kind = SearchFilters {
            author_kind: Some(MemberKind::Agent),
            ..Default::default()
        };
        assert!(!with_kind.is_empty());
    }
}
