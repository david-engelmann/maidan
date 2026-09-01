//! Cluster 309: Slack projector egress. A Maidan message in a linked thread is
//! relayed to its Slack channel; a Slack-sourced message (metadata tag) is not
//! echoed back (loop prevention); an unlinked thread is ignored.

use std::sync::{Arc, Mutex};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    slack::{route_message_to_slack, SlackError, SlackSender},
    AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewMessage, NewSlackChannelLink, NewThread, NewWorkspace,
};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct MockSender {
    sent: Mutex<Vec<(String, String)>>,
}

#[async_trait::async_trait]
impl SlackSender for MockSender {
    async fn post_message(&self, channel: &str, text: &str) -> Result<(), SlackError> {
        self.sent
            .lock()
            .unwrap()
            .push((channel.into(), text.into()));
        Ok(())
    }
}

async fn setup() -> (AppState, Arc<MockSender>, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    std::mem::forget(dir);
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    let sender = Arc::new(MockSender {
        sent: Mutex::new(Vec::new()),
    });
    state.attach_slack_sender(sender.clone());
    (state, sender, store)
}

async fn post(
    store: &dyn Store,
    thread: maidan_types::ThreadId,
    author: maidan_types::MemberId,
    body: &str,
    metadata: serde_json::Value,
) -> maidan_types::Message {
    store
        .post_message_with_event(
            NewMessage {
                thread_id: thread,
                author_id: author,
                body: body.into(),
                metadata,
                content: None,
            },
            None,
        )
        .await
        .unwrap()
        .0
}

#[tokio::test]
async fn egress_relays_a_linked_thread_message_and_skips_slack_sourced() {
    let (state, sender, store) = setup().await;
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("slack".into()),
        })
        .await
        .unwrap();
    store
        .link_slack_channel(NewSlackChannelLink {
            slack_channel_id: "C1".into(),
            workspace_id: ws.id,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: agent.id,
        })
        .await
        .unwrap();

    // A normal Maidan message in the linked thread is relayed to Slack.
    let m = post(
        store.as_ref(),
        thread.id,
        agent.id,
        "hi from maidan",
        json!({}),
    )
    .await;
    route_message_to_slack(&state, thread.id, &m).await;
    {
        let sent = sender.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0], ("C1".to_string(), "hi from maidan".to_string()));
    }

    // A Slack-sourced message (metadata tag) is NOT echoed back — no loop.
    let from_slack = post(
        store.as_ref(),
        thread.id,
        agent.id,
        "U9: from slack",
        json!({ "slack": { "user": "U9", "channel": "C1" } }),
    )
    .await;
    route_message_to_slack(&state, thread.id, &from_slack).await;
    assert_eq!(
        sender.sent.lock().unwrap().len(),
        1,
        "a Slack-sourced message is not relayed back to Slack"
    );

    // A message in an unlinked thread is ignored.
    let other = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("other".into()),
        })
        .await
        .unwrap();
    let m2 = post(store.as_ref(), other.id, agent.id, "unlinked", json!({})).await;
    route_message_to_slack(&state, other.id, &m2).await;
    assert_eq!(
        sender.sent.lock().unwrap().len(),
        1,
        "an unlinked thread does not relay to Slack"
    );
}
