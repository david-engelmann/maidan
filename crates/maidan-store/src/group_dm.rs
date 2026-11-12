//! Group DM helpers shared by the REST route and the MCP tool.

use maidan_types::{GroupDmConversationId, MemberId};

pub fn group_dm_conversation_id() -> GroupDmConversationId {
    GroupDmConversationId::new()
}

/// The members a caller opens a group DM with, the caller included. A 1:1
/// DM's opener is always one of its two members (the route takes the caller
/// as one side), so a group DM's opener is too: a list that leaves the caller
/// out gets them added first; a list that names them is unchanged.
pub fn with_opener(mut member_ids: Vec<MemberId>, opener: MemberId) -> Vec<MemberId> {
    if !member_ids.contains(&opener) {
        member_ids.insert(0, opener);
    }
    member_ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opener_is_added_once_and_only_when_missing() {
        let (a, b, c) = (
            MemberId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
        );
        assert_eq!(with_opener(vec![b, c], a), vec![a, b, c]);
        assert_eq!(with_opener(vec![b, a, c], a), vec![b, a, c]);
    }
}
