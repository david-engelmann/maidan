//! WS/MCP subscribe channel grant enforcement (Cluster 81.0).

use std::collections::HashSet;

use maidan_auth::AuthContext;
use maidan_router::resolve_thread_context;
use maidan_types::{ChannelId, EventFilter, WorkspaceId, DM_CHANNEL_NAME};

use crate::state::AppState;

pub async fn apply_subscribe_grants(
    state: &AppState,
    auth: &AuthContext,
    filter: &mut EventFilter,
) -> Result<(), String> {
    let Some(ws) = filter.workspace_id else {
        return Ok(());
    };
    let channels = state
        .store
        .list_channels(ws)
        .await
        .map_err(|e| format!("list channels: {e}"))?;
    let mut grants: HashSet<ChannelId> = filter
        .channel_grants
        .as_ref()
        .map(|v| v.iter().copied().collect())
        .unwrap_or_default();
    // Verify asserted private-channel grants against actual membership
    // (Cluster 163) — the client no longer merely *claims* access. Public and
    // the DM system channel pass; a private-channel grant the caller isn't a
    // member of is dropped, so the checks below deny that channel/thread and it
    // lands in `private_channel_deny`. Bypass callers keep all asserted grants.
    if !auth.bypass {
        let mut verified: HashSet<ChannelId> = HashSet::new();
        for cid in grants.iter().copied() {
            match channels.iter().find(|c| c.id == cid) {
                Some(c) if !c.private || c.name == DM_CHANNEL_NAME => {
                    verified.insert(cid);
                }
                Some(_) => {
                    if state
                        .store
                        .channel_is_member(cid, auth.member_id)
                        .await
                        .map_err(|e| format!("channel_is_member: {e}"))?
                    {
                        verified.insert(cid);
                    }
                }
                None => {}
            }
        }
        grants = verified;
    }
    if filter.dm_conversation_id.is_some() {
        if let Some(th) = filter.thread_id {
            let ctx = resolve_thread_context(state.store.as_ref(), th)
                .await
                .map_err(|e| format!("{e:?}"))?;
            grants.insert(ctx.channel_id);
        }
    }
    if let Some(ch) = filter.channel_id {
        let channel = channels
            .iter()
            .find(|c| c.id == ch)
            .ok_or_else(|| "channel not found in workspace".to_string())?;
        if channel.private && !grants.contains(&ch) {
            return Err("private channel requires channel_grants entry".into());
        }
    }
    if let Some(th) = filter.thread_id {
        let ctx = resolve_thread_context(state.store.as_ref(), th)
            .await
            .map_err(|e| format!("{e:?}"))?;
        let channel = channels
            .iter()
            .find(|c| c.id == ctx.channel_id)
            .ok_or_else(|| "thread channel not found".to_string())?;
        if channel.private && !grants.contains(&ctx.channel_id) {
            return Err("private channel requires channel_grants entry".into());
        }
    }
    for channel in channels.iter().filter(|c| c.private) {
        if !grants.contains(&channel.id) {
            filter.private_channel_deny.insert(channel.id);
        }
    }
    if !grants.is_empty() {
        filter.channel_event_allow = Some(grants);
    }
    Ok(())
}

pub fn workspace_filter(ws: WorkspaceId, channel_grants: &[ChannelId]) -> EventFilter {
    EventFilter {
        workspace_id: Some(ws),
        channel_grants: Some(channel_grants.to_vec()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{atomic::AtomicI64, Arc};

    use maidan_artifacts::LocalFsStore;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::{ChannelMemberRole, MemberKind, NewChannel, NewMember, NewWorkspace};
    use sqlx::sqlite::SqlitePoolOptions;

    use crate::state::FederationRuntime;

    async fn state_with_auth() -> (AppState, WorkspaceId, maidan_types::Member, ChannelId) {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
        let ws = store
            .create_workspace(NewWorkspace { name: "w".into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: ws.id,
                handle: "bob".into(),
                display_name: None,
                kind: MemberKind::Human,
            })
            .await
            .unwrap();
        let ch = store
            .create_channel(NewChannel {
                workspace_id: ws.id,
                name: "secret".into(),
                topic: None,
                private: true,
            })
            .await
            .unwrap();
        let search: Arc<dyn maidan_search::Search> =
            Arc::new(maidan_search::SqliteSearch::new(pool));
        let state = AppState::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_bus::InMemoryBus::new()),
            search,
            Arc::new(maidan_search::HashV1Provider),
            false, // auth ENABLED — verification runs
            false,
            FederationRuntime::new(true, None),
            Arc::new(AtomicI64::new(0)),
            None,
        );
        (state, ws.id, member, ch.id)
    }

    #[tokio::test]
    async fn asserted_private_grant_is_dropped_for_non_members() {
        let (state, ws, bob, ch) = state_with_auth().await;
        let auth = AuthContext::from_session(bob.id, ws, vec![]);

        // Bob asserts a grant for a private channel he isn't in → dropped, and
        // the channel is denied.
        let mut filter = EventFilter {
            workspace_id: Some(ws),
            channel_grants: Some(vec![ch]),
            ..Default::default()
        };
        apply_subscribe_grants(&state, &auth, &mut filter)
            .await
            .unwrap();
        assert!(filter.private_channel_deny.contains(&ch));
        assert!(filter
            .channel_event_allow
            .as_ref()
            .map(|g| !g.contains(&ch))
            .unwrap_or(true));

        // After Bob is added to the channel, the grant is honored.
        state
            .store
            .add_channel_member(ch, bob.id, ChannelMemberRole::Member)
            .await
            .unwrap();
        let mut filter = EventFilter {
            workspace_id: Some(ws),
            channel_grants: Some(vec![ch]),
            ..Default::default()
        };
        apply_subscribe_grants(&state, &auth, &mut filter)
            .await
            .unwrap();
        assert!(!filter.private_channel_deny.contains(&ch));
        assert!(filter.channel_event_allow.as_ref().unwrap().contains(&ch));
    }
}
