//! Direct-message conversation helpers.

use maidan_types::{DmConversationId, MemberId, DM_CHANNEL_NAME};

pub fn ordered_members(a: MemberId, b: MemberId) -> Result<(MemberId, MemberId), &'static str> {
    if a == b {
        return Err("cannot open a DM with yourself");
    }
    if a.0 < b.0 {
        Ok((a, b))
    } else {
        Ok((b, a))
    }
}

pub fn dm_channel_name() -> &'static str {
    DM_CHANNEL_NAME
}

pub fn dm_conversation_id() -> DmConversationId {
    DmConversationId::new()
}
