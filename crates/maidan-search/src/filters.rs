//! Search facets (v1.2.2), and the reader a search is run for.

use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, MemberKind};

/// Facets applied in addition to the workspace scope and FTS match.
///
/// There is no `Default`: every search is run for a reader, so a caller cannot
/// build filters that forget who is asking and read every thread. Start from
/// [`SearchFilters::for_reader`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchFilters {
    pub author_id: Option<MemberId>,
    pub channel_id: Option<ChannelId>,
    /// Restrict to messages whose author has this [`MemberKind`].
    pub author_kind: Option<MemberKind>,
    /// Date-range facet: only messages posted at/after this instant. Inclusive
    /// lower bound on `posted_at`.
    pub after: Option<DateTime<Utc>>,
    /// Date-range facet: only messages posted strictly before this instant.
    /// Exclusive upper bound on `posted_at` — a half-open `[after, before)`
    /// window so adjacent day-ranges don't double-count a midnight boundary.
    pub before: Option<DateTime<Utc>>,
    /// RBAC pre-filter: channels whose messages must be excluded at the query
    /// level — the private channels the caller isn't a member of. Not a
    /// user-settable facet; the server computes it so inaccessible hits never
    /// crowd out the requested `limit`. Empty = no restriction.
    pub deny_channels: Vec<ChannelId>,
    /// Who is searching. Every query keeps only the threads this member may
    /// read (`thread_access::readable_thread`): threads of their own
    /// workspace, a private channel's only if they belong to it, a DM's only
    /// if they are in it. It is filtered in the query so hidden rows never
    /// crowd out the limit.
    pub reader_id: MemberId,
}

impl SearchFilters {
    /// Every hit `reader_id` may read, with no facet set.
    pub fn for_reader(reader_id: MemberId) -> Self {
        Self {
            author_id: None,
            channel_id: None,
            author_kind: None,
            after: None,
            before: None,
            deny_channels: Vec::new(),
            reader_id,
        }
    }

    /// `true` when no **user** facet is set. The reader and the RBAC
    /// `deny_channels` pre-filter are deliberately excluded: the query applies
    /// them regardless.
    pub fn is_empty(&self) -> bool {
        self.author_id.is_none()
            && self.channel_id.is_none()
            && self.author_kind.is_none()
            && self.after.is_none()
            && self.before.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::{ChannelId, MemberId, MemberKind};

    fn reader() -> MemberId {
        MemberId(uuid::Uuid::now_v7())
    }

    #[test]
    fn is_empty_when_no_facets_set() {
        assert!(SearchFilters::for_reader(reader()).is_empty());
        let denied = SearchFilters {
            deny_channels: vec![ChannelId(uuid::Uuid::now_v7())],
            ..SearchFilters::for_reader(reader())
        };
        assert!(denied.is_empty(), "the RBAC pre-filter is not a user facet");
    }

    #[test]
    fn is_not_empty_when_any_facet_set() {
        let with_author = SearchFilters {
            author_id: Some(MemberId(uuid::Uuid::now_v7())),
            ..SearchFilters::for_reader(reader())
        };
        assert!(!with_author.is_empty());

        let with_channel = SearchFilters {
            channel_id: Some(ChannelId(uuid::Uuid::now_v7())),
            ..SearchFilters::for_reader(reader())
        };
        assert!(!with_channel.is_empty());

        let with_kind = SearchFilters {
            author_kind: Some(MemberKind::Agent),
            ..SearchFilters::for_reader(reader())
        };
        assert!(!with_kind.is_empty());

        let with_after = SearchFilters {
            after: Some(chrono::Utc::now()),
            ..SearchFilters::for_reader(reader())
        };
        assert!(!with_after.is_empty());

        let with_before = SearchFilters {
            before: Some(chrono::Utc::now()),
            ..SearchFilters::for_reader(reader())
        };
        assert!(!with_before.is_empty());
    }
}
