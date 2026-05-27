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
