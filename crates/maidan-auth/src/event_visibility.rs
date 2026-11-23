//! Which logged events a caller may read.
//!
//! The event log holds every event in a workspace, message bodies included. A
//! member reading it back sees what they could see anywhere else: nothing from a
//! private channel they are not in, and nothing from a DM they are not part of.
//! This is the one rule every reader of events applies: the backfill, the
//! live streams (`/ws/subscribe`, `/mcp/stream`, `/agui/stream`) on replay and
//! on live delivery, webhook delivery, and the per-event check a one-shot MCP
//! waiter (`wait_for_event`) applies to what it is handed live. A stream's
//! channel grants only narrow what it asks for and never lend access.
//! Federation peers read the whole log.
//!
//! Withdrawn words need no filter here: withdrawal shreds the message's
//! content key, so its posted and edited events read back as sealed
//! ciphertext with an empty body ([`maidan_types::content_seal`]).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::{AuthContext, AuthError};
use maidan_store::{Store, StoreError};
use maidan_types::{ChannelId, Event, StoredEvent, ThreadId, DM_CHANNEL_NAME};

pub struct EventVisibility<'a> {
    store: &'a dyn Store,
    auth: &'a AuthContext,
    /// How long a decision is reused. `None` keeps it for the reader's life,
    /// which suits a one-shot page; a stream that runs for hours sets one so
    /// a removal from a private channel or a DM still takes effect.
    lifetime: Option<Duration>,
    threads: HashMap<ThreadId, (bool, Instant)>,
    channels: HashMap<ChannelId, (bool, Instant)>,
}

impl<'a> EventVisibility<'a> {
    pub fn new(store: &'a dyn Store, auth: &'a AuthContext) -> Self {
        Self {
            store,
            auth,
            lifetime: None,
            threads: HashMap::new(),
            channels: HashMap::new(),
        }
    }

    /// Reuse each decision for `lifetime` at most, so a burst of events on one
    /// thread costs one lookup and a change of membership still shows up
    /// within `lifetime`.
    pub fn with_lifetime(mut self, lifetime: Duration) -> Self {
        self.lifetime = Some(lifetime);
        self
    }

    fn fresh(&self, at: Instant) -> bool {
        self.lifetime.is_none_or(|lifetime| at.elapsed() < lifetime)
    }

    /// Whether the caller may see a live `event`. It must be in the caller's
    /// workspace, so an event that names none (a reference or an artifact
    /// upsert) is never shown. Then it follows [`Self::allows_in`] by where it
    /// happened: every message-scoped event (a reaction, a vote, a mention, a
    /// pin) carries its thread, so it is as visible as the thread, and a DM's
    /// follow the conversation's participants. An error is not a yes: callers
    /// treat it as hidden.
    pub async fn allows_event(&mut self, event: &Event) -> Result<bool, StoreError> {
        if event.workspace_id() != Some(self.auth.workspace_id()) {
            return Ok(false);
        }
        self.allows_in(event.thread_id(), event.channel_id()).await
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
        if let Some(thread_id) = thread_id {
            if let Some(&(seen, at)) = self.threads.get(&thread_id)
                && self.fresh(at)
            {
                return Ok(seen);
            }
            let allowed = settle(crate::can_access_thread(self.store, self.auth, thread_id).await)?;
            self.threads.insert(thread_id, (allowed, Instant::now()));
            return Ok(allowed);
        }
        if let Some(channel_id) = channel_id {
            if let Some(&(seen, at)) = self.channels.get(&channel_id)
                && self.fresh(at)
            {
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
            self.channels.insert(channel_id, (allowed, Instant::now()));
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

    /// A logged event names its thread, and the thread's access rule decides:
    /// hidden in a private channel the reader is not in, shown once they are.
    #[tokio::test]
    async fn a_logged_event_follows_its_threads_access() {
        let (store, auth, public_own, _, _) = seed().await;
        let inner = channel(&store, auth.workspace_id(), "inner", true).await;
        let thread = |channel_id| maidan_types::NewThread {
            channel_id,
            parent_thread_id: None,
            title: Some("logged".into()),
            description: None,
            created_by: None,
        };
        let (_, shown) = store
            .create_thread_with_event(thread(public_own))
            .await
            .expect("public thread");
        let (_, hidden) = store
            .create_thread_with_event(thread(inner))
            .await
            .expect("private thread");
        let mut vis = EventVisibility::new(&store, &auth);
        assert!(vis.allows(&shown).await.expect("shown"));
        assert!(!vis.allows(&hidden).await.expect("hidden"));
        store
            .add_channel_member(
                inner,
                auth.member_id(),
                maidan_types::ChannelMemberRole::Member,
            )
            .await
            .expect("join");
        assert!(EventVisibility::new(&store, &auth)
            .allows(&hidden)
            .await
            .expect("after joining"));
    }

    #[tokio::test]
    async fn a_private_channel_in_another_workspace_is_not_visible() {
        let (store, auth, _, _, private_other) = seed().await;
        let mut vis = EventVisibility::new(&store, &auth);
        assert!(!vis.allows_in(None, Some(private_other)).await.unwrap());
    }

    /// A decision is reused for its lifetime and no longer, so leaving a
    /// private channel ends a long-lived stream's view of it.
    #[tokio::test]
    async fn a_decision_is_reused_only_within_its_lifetime() {
        let (store, auth, _, _, _) = seed().await;
        let inner = channel(&store, auth.workspace_id(), "inner", true).await;
        store
            .add_channel_member(
                inner,
                auth.member_id(),
                maidan_types::ChannelMemberRole::Member,
            )
            .await
            .expect("join");
        let mut vis = EventVisibility::new(&store, &auth).with_lifetime(Duration::from_millis(200));
        assert!(vis.allows_in(None, Some(inner)).await.unwrap());
        store
            .remove_channel_member(inner, auth.member_id())
            .await
            .expect("leave");
        assert!(
            vis.allows_in(None, Some(inner)).await.unwrap(),
            "inside the lifetime the earlier answer stands"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !vis.allows_in(None, Some(inner)).await.unwrap(),
            "past the lifetime the membership is read again"
        );
    }

    /// An event that names no workspace cannot be shown to be the caller's.
    #[tokio::test]
    async fn an_event_without_a_workspace_is_not_visible() {
        let (store, auth, _, _, _) = seed().await;
        let event = Event::ReferenceAdded {
            occurred_at: chrono::Utc::now(),
            reference: maidan_types::Reference {
                id: uuid::Uuid::now_v7(),
                src_kind: maidan_types::RefSide::Thread,
                src_id: uuid::Uuid::now_v7(),
                dst_kind: maidan_types::RefSide::Thread,
                dst_id: uuid::Uuid::now_v7(),
                relation: maidan_types::RelationKind::Supports,
                created_at: chrono::Utc::now(),
            },
        };
        let mut vis = EventVisibility::new(&store, &auth);
        assert!(!vis.allows_event(&event).await.unwrap());
    }
}
