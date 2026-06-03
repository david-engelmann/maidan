//! Group DM conversation id helper.

use maidan_types::GroupDmConversationId;

pub fn group_dm_conversation_id() -> GroupDmConversationId {
    GroupDmConversationId::new()
}
