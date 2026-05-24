//! Indexer integration tests against an in-memory bus. The server-
//! coupled variant lives in `maidan-server/tests/indexer_e2e.rs` to
//! avoid a dev-dep cycle (maidan-server depends on maidan-search).

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use maidan_bus::{EventBus, InMemoryBus};
use maidan_search::{Indexer, LoggingHandler};
use maidan_types::{
    BusEnvelope, ChannelId, Event, EventKind, MemberId, Message, MessageId, ThreadId, Workspace,
    WorkspaceId,
};

fn make_message(thread_id: ThreadId, author_id: MemberId, body: &str) -> Message {
    Message {
        id: MessageId(uuid::Uuid::new_v4()),
        thread_id,
        author_id,
        body: body.into(),
        metadata: serde_json::json!({}),
        posted_at: Utc::now(),
        edited_at: None,
        tombstoned_at: None,
    }
}

#[tokio::test]
async fn indexer_observes_message_posted_within_500ms() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::with_capacity(256));
    let handler = Arc::new(LoggingHandler::default());
    let indexer = Indexer::new(bus.clone(), handler.clone()).spawn();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_id = WorkspaceId(uuid::Uuid::new_v4());
    let ch_id = ChannelId(uuid::Uuid::new_v4());
    let th_id = ThreadId(uuid::Uuid::new_v4());
    let author = MemberId(uuid::Uuid::new_v4());
    let msg = make_message(th_id, author, "hello indexer");

    bus.publish(BusEnvelope::synthetic(Event::MessagePosted {
        occurred_at: Utc::now(),
        workspace_id: ws_id,
        channel_id: ch_id,
        thread_id: th_id,
        message: msg,
    }))
    .await
    .unwrap();

    let observed = handler
        .wait_for(Duration::from_millis(500), |log| !log.is_empty())
        .await
        .expect("indexer did not observe event within 500ms");
    assert_eq!(observed, vec![EventKind::MessagePosted]);
    indexer.shutdown().await;
}

#[tokio::test]
async fn indexer_filters_out_non_message_events() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::with_capacity(256));
    let handler = Arc::new(LoggingHandler::default());
    let indexer = Indexer::new(bus.clone(), handler.clone()).spawn();
    tokio::time::sleep(Duration::from_millis(50)).await;

    bus.publish(BusEnvelope::synthetic(Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: "noise".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    }))
    .await
    .unwrap();

    bus.publish(BusEnvelope::synthetic(Event::MessageTombstoned {
        occurred_at: Utc::now(),
        workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
        channel_id: ChannelId(uuid::Uuid::new_v4()),
        thread_id: ThreadId(uuid::Uuid::new_v4()),
        message_id: MessageId(uuid::Uuid::new_v4()),
    }))
    .await
    .unwrap();

    let observed = handler
        .wait_for(Duration::from_millis(500), |log| {
            log.contains(&EventKind::MessageTombstoned)
        })
        .await
        .expect("indexer did not see tombstone in 500 ms");

    assert!(observed.contains(&EventKind::MessageTombstoned));
    assert!(
        !observed.contains(&EventKind::WorkspaceCreated),
        "WorkspaceCreated should be filtered out by the indexer's subscription"
    );

    indexer.shutdown().await;
}

#[tokio::test]
async fn indexer_clean_shutdown() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::with_capacity(256));
    let handler = Arc::new(LoggingHandler::default());
    let indexer = Indexer::new(bus, handler).spawn();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let shutdown_result =
        tokio::time::timeout(Duration::from_millis(500), indexer.shutdown()).await;
    assert!(
        shutdown_result.is_ok(),
        "indexer should shut down within 500 ms"
    );
}
