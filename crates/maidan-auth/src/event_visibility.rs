//! Which logged events a caller may read.
//!
//! The event log holds every event in a workspace, message bodies included. A
//! member reading it back sees what they could see anywhere else: nothing from a
//! private channel they are not in, and nothing from a DM they are not part of.
//! The live streams apply the same rule (`subscribe_grants`); this is the
//! backfill's half, and the per-event check a one-shot MCP waiter
//! (`wait_for_event`) applies to what it is handed live. Federation peers and
//! bypass callers read the whole log.
//!
//! Withdrawn words need no filter here: withdrawal shreds the message's
//! content key, so its posted and edited events read back as sealed
//! ciphertext with an empty body ([`maidan_types::content_seal`]).

use std::collections::HashMap;

use crate::{AuthContext, AuthError};
use maidan_store::{Store, StoreError};
use maidan_types::{ChannelId, StoredEvent, ThreadId, DM_CHANNEL_NAME};

pub struct EventVisibility<'a> {
    store: &'a dyn Store,
    auth: &'a AuthContext,
    threads: HashMap<ThreadId, bool>,
    channels: HashMap<ChannelId, bool>,
}

impl<'a> EventVisibility<'a> {
    pub fn new(store: &'a dyn Store, auth: &'a AuthContext) -> Self {
        Self {
            store,
            auth,
            threads: HashMap::new(),
            channels: HashMap::new(),
        }
    }

    /// Whether the caller may read `event`. A thread's event follows the
    /// thread's access rule, which is DM-participant aware. A channel's event
    /// without a thread follows channel membership, and the shared DM channel's
    /// are never shown: DM access is per conversation, and those events name
    /// none. Events of neither are workspace-wide.
    pub async fn allows(&mut self, event: &StoredEvent) -> Result<bool, StoreError> {
        self.allows_in(event.thread_id, event.channel_id).await
    }

    /// [`Self::allows`] for an event known by where it happened, as a live
    /// event off the bus is.
    pub async fn allows_in(
        &mut self,
        thread_id: Option<ThreadId>,
        channel_id: Option<ChannelId>,
    ) -> Result<bool, StoreError> {
        if self.auth.bypass {
            return Ok(true);
        }
        if let Some(thread_id) = thread_id {
            if let Some(&seen) = self.threads.get(&thread_id) {
                return Ok(seen);
            }
            let allowed = settle(crate::can_access_thread(self.store, self.auth, thread_id).await)?;
            self.threads.insert(thread_id, allowed);
            return Ok(allowed);
        }
        if let Some(channel_id) = channel_id {
            if let Some(&seen) = self.channels.get(&channel_id) {
                return Ok(seen);
            }
            let allowed = match self.store.get_channel(channel_id).await {
                // A channel id from another tenant is never visible, public
                // or not: the membership checks below assume this workspace.
                Ok(channel) if channel.workspace_id != self.auth.workspace_id() => false,
                Ok(channel) if channel.name == DM_CHANNEL_NAME => false,
                Ok(channel) if channel.private => {
                    self.store
                        .channel_is_member(channel_id, self.auth.member_id())
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

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_store::{prelude::*, run_sqlite_migrations};
    use maidan_types::{MemberKind, NewChannel, NewMember, NewWorkspace};

    async fn sqlite() -> SqliteStore {
        let pool = maidan_search::sqlite_pool_options()
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("pragma");
        run_sqlite_migrations(&pool).await.expect("migrate");
        SqliteStore::for_tests(pool)
    }

    async fn channel(
        store: &SqliteStore,
        workspace_id: maidan_types::WorkspaceId,
        name: &str,
        private: bool,
    ) -> ChannelId {
        store
            .create_channel(NewChannel {
                workspace_id,
                name: name.into(),
                topic: None,
                private,
            })
            .await
            .expect("channel")
            .id
    }

    /// A reader in workspace A, A's public channel, and B's public and
    /// private channels.
    async fn seed() -> (SqliteStore, AuthContext, ChannelId, ChannelId, ChannelId) {
        let store = sqlite().await;
        let ws_a = store
            .create_workspace(NewWorkspace { name: "a".into() })
            .await
            .expect("ws a");
        let ws_b = store
            .create_workspace(NewWorkspace { name: "b".into() })
            .await
            .expect("ws b");
        let member = store
            .create_member(NewMember {
                workspace_id: ws_a.id,
                handle: "reader".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .expect("member");
        let public_own = channel(&store, ws_a.id, "general", false).await;
        let public_other = channel(&store, ws_b.id, "general", false).await;
        let private_other = channel(&store, ws_b.id, "secret", true).await;
        let auth = AuthContext::from_session(
            member.id,
            ws_a.id,
            vec![crate::capability::WORKSPACE_READ.to_string()],
        );
        (store, auth, public_own, public_other, private_other)
    }

    #[tokio::test]
    async fn a_public_channel_in_another_workspace_is_not_visible() {
        let (store, auth, _, public_other, _) = seed().await;
        let mut vis = EventVisibility::new(&store, &auth);
        assert!(!vis.allows_in(None, Some(public_other)).await.unwrap());
    }

    #[tokio::test]
    async fn a_public_channel_in_the_callers_workspace_is_visible() {
        let (store, auth, public_own, _, _) = seed().await;
        let mut vis = EventVisibility::new(&store, &auth);
        assert!(vis.allows_in(None, Some(public_own)).await.unwrap());
    }

    #[tokio::test]
    async fn a_private_channel_in_another_workspace_is_not_visible() {
        let (store, auth, _, _, private_other) = seed().await;
        let mut vis = EventVisibility::new(&store, &auth);
        assert!(!vis.allows_in(None, Some(private_other)).await.unwrap());
    }
}
