//! Which logged events a caller may read.
//!
//! The event log holds every event in a workspace, message bodies included. A
//! member reading it back sees what they could see anywhere else: nothing from a
//! private channel they are not in, and nothing from a DM they are not part of.
//! The live streams apply the same rule (`subscribe_grants`); this is the
//! backfill's half. Federation peers and bypass callers read the whole log.
//!
//! A message withdrawn since it was logged reads back withdrawn: its posted and
//! edited events keep their place in the log, with the words blanked, as the
//! live row is. The whole log, words included, stays with the admin tier.

use std::collections::HashMap;

use maidan_auth::{AuthContext, AuthError};
use maidan_store::{Store, StoreError};
use maidan_types::{ChannelId, EventKind, MessageId, StoredEvent, ThreadId, DM_CHANNEL_NAME};

pub struct EventVisibility<'a> {
    store: &'a dyn Store,
    auth: &'a AuthContext,
    threads: HashMap<ThreadId, bool>,
    channels: HashMap<ChannelId, bool>,
    withdrawn: HashMap<MessageId, bool>,
}

impl<'a> EventVisibility<'a> {
    pub fn new(store: &'a dyn Store, auth: &'a AuthContext) -> Self {
        Self {
            store,
            auth,
            threads: HashMap::new(),
            channels: HashMap::new(),
            withdrawn: HashMap::new(),
        }
    }

    /// Whether the caller may read `event`. A thread's event follows the
    /// thread's access rule, which is DM-participant aware. A channel's event
    /// without a thread follows channel membership, and the shared DM channel's
    /// are never shown: DM access is per conversation, and those events name
    /// none. Events of neither are workspace-wide.
    pub async fn allows(&mut self, event: &StoredEvent) -> Result<bool, StoreError> {
        if self.auth.bypass {
            return Ok(true);
        }
        if let Some(thread_id) = event.thread_id {
            if let Some(&seen) = self.threads.get(&thread_id) {
                return Ok(seen);
            }
            let allowed =
                settle(maidan_auth::can_access_thread(self.store, self.auth, thread_id).await)?;
            self.threads.insert(thread_id, allowed);
            return Ok(allowed);
        }
        if let Some(channel_id) = event.channel_id {
            if let Some(&seen) = self.channels.get(&channel_id) {
                return Ok(seen);
            }
            let allowed = match self.store.get_channel(channel_id).await {
                Ok(channel) if channel.name == DM_CHANNEL_NAME => false,
                Ok(channel) if channel.private => {
                    self.store
                        .channel_is_member(channel_id, self.auth.member_id)
                        .await?
                }
                Ok(_) => true,
                Err(StoreError::NotFound) => false,
                Err(e) => return Err(e),
            };
            self.channels.insert(channel_id, allowed);
            return Ok(allowed);
        }
        Ok(true)
    }
}

impl EventVisibility<'_> {
    /// Blank the words of a message withdrawn since this event was logged. A
    /// message that no longer exists (purged) counts as withdrawn.
    pub async fn redact_withdrawn(&mut self, event: &mut StoredEvent) -> Result<(), StoreError> {
        if self.auth.bypass
            || !matches!(
                event.kind,
                EventKind::MessagePosted | EventKind::MessageEdited
            )
        {
            return Ok(());
        }
        let Some(id) = event.payload["message"]["id"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(MessageId)
        else {
            return Ok(());
        };
        let withdrawn = match self.withdrawn.get(&id) {
            Some(&known) => known,
            None => {
                let known = match self.store.get_message(id).await {
                    Ok(message) => message.tombstoned_at.is_some(),
                    Err(StoreError::NotFound) => true,
                    Err(e) => return Err(e),
                };
                self.withdrawn.insert(id, known);
                known
            }
        };
        if withdrawn {
            let message = &mut event.payload["message"];
            message["body"] = serde_json::Value::String(String::new());
            message["content"] = serde_json::Value::Null;
        }
        Ok(())
    }
}

/// A thread or channel that no longer exists shows nothing; a store failure
/// is still a failure.
fn settle(result: Result<bool, AuthError>) -> Result<bool, StoreError> {
    match result {
        Ok(allowed) => Ok(allowed),
        Err(AuthError::Store(StoreError::NotFound)) => Ok(false),
        Err(AuthError::Store(e)) => Err(e),
        Err(AuthError::Unauthorized | AuthError::Forbidden(_)) => Ok(false),
    }
}
