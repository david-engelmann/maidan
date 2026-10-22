//! A result delivery that is skipped or fails says so once on the event
//! stream (`result_delivery_failed`), so a relay can tell the task's thread
//! where the result did not go. The event is written with the row's status
//! change, in the same transaction, and published by whoever recorded it.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{BusItem, EventBus, EventStream, InMemoryBus};
use maidan_server::{
    egress_worker, notification_router,
    slack::{SlackError, SlackSender},
    AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    status, ChannelId, EgressSurface, Event, EventFilter, EventKind, ExternalRef, MemberId,
    MemberKind, NewChannel, NewEgressTarget, NewMember, NewThread, NewWorkspace, ThreadId,
    WorkspaceId, WAITER_RESULT_SCHEMA,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tokio_stream::StreamExt;

const SLACK_CHANNEL: &str = "C0123ABCDEF";

/// A Slack that accepts every post, or refuses every one with a credential
/// error the worker treats as a misconfiguration (dead-lettered at once).
struct FakeSlack {
    refuse: bool,
    posts: Mutex<usize>,
}

#[async_trait::async_trait]
impl SlackSender for FakeSlack {
    async fn post_message(
        &self,
        channel: &str,
        _text: &str,
        _thread_ts: Option<&str>,
    ) -> Result<Option<ExternalRef>, SlackError> {
        if self.refuse {
            return Err(SlackError::Api("invalid_auth".into()));
        }
        let mut posts = self.posts.lock().unwrap();
        *posts += 1;
        Ok(Some(ExternalRef::Slack {
            channel_id: channel.into(),
            ts: format!("1759500001.{:06}", *posts),
        }))
    }

    async fn update_message(&self, _: &str, _: &str, _: &str) -> Result<(), SlackError> {
        Ok(())
    }
}

struct Tenant {
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    member_id: MemberId,
}

struct Harness {
    store: Arc<dyn Store>,
    state: AppState,
    bus: Arc<InMemoryBus>,
    a: Tenant,
    b: Tenant,
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
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
            title: Some(name.into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    Tenant {
        workspace_id: ws.id,
        channel_id: channel.id,
        thread_id: thread.id,
        member_id: member.id,
    }
}

async fn harness(slack: Option<FakeSlack>) -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus.clone(), search);
    if let Some(slack) = slack {
        state.attach_slack_sender(Arc::new(slack));
    }
    std::mem::forget(dir);
    let a = tenant(store.as_ref(), "tenant-a").await;
    let b = tenant(store.as_ref(), "tenant-b").await;
    Harness {
        store,
        state,
        bus,
        a,
        b,
    }
}

fn slack_result(summary: &str) -> Value {
    json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "example.review.result/1",
        "status": "reviewed",
        "deliver_to": [{"surface": "slack", "channel": SLACK_CHANNEL}],
        "rendered": summary,
        "summary": summary,
    })
}

impl Harness {
    async fn bless_slack(&self, t: &Tenant) {
        self.store
            .allow_egress_target(NewEgressTarget {
                workspace_id: t.workspace_id,
                surface: EgressSurface::Slack,
                selector: SLACK_CHANNEL.into(),
            })
            .await
            .unwrap();
    }

    async fn route(&self, t: &Tenant, result: &Value, log_id: i64) {
        self.store
            .set_thread_result(t.thread_id, t.member_id, result)
            .await
            .unwrap();
        notification_router::route_event(
            &self.state,
            log_id,
            &Event::ThreadResultSet {
                occurred_at: Utc::now(),
                workspace_id: t.workspace_id,
                channel_id: t.channel_id,
                thread_id: t.thread_id,
                produced_by: t.member_id,
            },
        )
        .await
        .unwrap();
    }

    /// The `result_delivery_failed` rows in `t`'s event log, oldest first.
    async fn logged(&self, t: &Tenant) -> Vec<Event> {
        self.store
            .list_events_matching(
                t.workspace_id,
                None,
                Some(t.thread_id),
                &[EventKind::ResultDeliveryFailed],
                0,
                100,
            )
            .await
            .unwrap()
            .into_iter()
            .map(|stored| stored.opened_event().unwrap())
            .collect()
    }

    async fn subscribe(&self, workspace_id: WorkspaceId) -> EventStream {
        self.bus
            .subscribe(EventFilter {
                workspace_id: Some(workspace_id),
                kinds: Some(HashSet::from([EventKind::ResultDeliveryFailed])),
                ..EventFilter::default()
            })
            .await
            .unwrap()
    }
}

/// Every event `stream` has buffered, waiting briefly for stragglers.
async fn drain(stream: &mut EventStream) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Ok(Some(item)) = tokio::time::timeout(Duration::from_millis(300), stream.next()).await
    {
        if let BusItem::Event(envelope) = item {
            seen.push(envelope.event);
        }
    }
    seen
}

fn assert_says(event: &Event, t: &Tenant, want_status: &str, why: &str) {
    let Event::ResultDeliveryFailed {
        workspace_id,
        channel_id,
        thread_id,
        surface,
        selector,
        status,
        last_error,
        ..
    } = event
    else {
        panic!("not a result_delivery_failed: {event:?}");
    };
    assert_eq!(*workspace_id, t.workspace_id, "the thread's own workspace");
    assert_eq!(*channel_id, t.channel_id);
    assert_eq!(*thread_id, t.thread_id);
    assert_eq!(surface, "slack");
    assert_eq!(selector, SLACK_CHANNEL);
    assert_eq!(status, want_status);
    let last_error = last_error.as_deref().unwrap_or_default();
    assert!(last_error.contains(why), "{why:?} not in {last_error:?}");
}

#[tokio::test]
async fn a_skipped_delivery_publishes_one_event_naming_its_target() {
    let h = harness(None).await;
    let mut a_stream = h.subscribe(h.a.workspace_id).await;
    let mut b_stream = h.subscribe(h.b.workspace_id).await;

    // Unblessed: the router records the target as skipped.
    h.route(&h.a, &slack_result("first"), 1).await;

    let row = &h.store.list_result_deliveries(h.a.thread_id).await.unwrap()[0];
    assert_eq!(row.status, status::SKIPPED);
    let published = drain(&mut a_stream).await;
    assert_eq!(published.len(), 1, "{published:?}");
    assert_says(&published[0], &h.a, status::SKIPPED, "allowlist");
    let logged = h.logged(&h.a).await;
    let json = |events: &[Event]| serde_json::to_value(events).unwrap();
    assert_eq!(
        json(&logged),
        json(&published),
        "the log holds what was published"
    );
    assert!(
        drain(&mut b_stream).await.is_empty(),
        "another workspace's stream never hears it"
    );
    assert!(h.logged(&h.b).await.is_empty());
}

#[tokio::test]
async fn a_rearmed_delivery_skipped_again_says_so_again() {
    let h = harness(None).await;
    let mut stream = h.subscribe(h.a.workspace_id).await;
    h.route(&h.a, &slack_result("first"), 1).await;
    // A new revision of the result re-arms the row (pending) and the router
    // skips it again: a second transition, a second event.
    tokio::time::sleep(Duration::from_millis(5)).await;
    h.route(&h.a, &slack_result("second"), 2).await;

    let published = drain(&mut stream).await;
    assert_eq!(published.len(), 2, "{published:?}");
    for event in &published {
        assert_says(event, &h.a, status::SKIPPED, "allowlist");
    }
    assert_eq!(h.logged(&h.a).await.len(), 2);
}

#[tokio::test]
async fn recording_the_same_outcome_twice_says_it_once() {
    let h = harness(None).await;
    h.route(&h.a, &slack_result("first"), 1).await;
    let row = h.store.list_result_deliveries(h.a.thread_id).await.unwrap()[0].clone();

    let again = h
        .store
        .mark_result_delivery_skipped(row.id, "still not blessed")
        .await
        .unwrap();

    assert!(again.is_none(), "already skipped: nothing new to say");
    assert_eq!(h.logged(&h.a).await.len(), 1);
    let row = &h.store.list_result_deliveries(h.a.thread_id).await.unwrap()[0];
    assert_eq!(row.last_error.as_deref(), Some("still not blessed"));
}

#[tokio::test]
async fn a_dead_lettered_delivery_publishes_one_failed_event() {
    let h = harness(Some(FakeSlack {
        refuse: true,
        posts: Mutex::new(0),
    }))
    .await;
    h.bless_slack(&h.a).await;
    let mut stream = h.subscribe(h.a.workspace_id).await;

    h.route(&h.a, &slack_result("first"), 1).await;
    egress_worker::sweep_once(&h.state).await;

    let row = &h.store.list_result_deliveries(h.a.thread_id).await.unwrap()[0];
    assert_eq!(row.status, status::FAILED);
    let published = drain(&mut stream).await;
    assert_eq!(published.len(), 1, "{published:?}");
    assert_says(&published[0], &h.a, status::FAILED, "invalid_auth");
    assert_eq!(h.logged(&h.a).await.len(), 1);
}

#[tokio::test]
async fn a_delivered_result_publishes_nothing() {
    let h = harness(Some(FakeSlack {
        refuse: false,
        posts: Mutex::new(0),
    }))
    .await;
    h.bless_slack(&h.a).await;
    let mut stream = h.subscribe(h.a.workspace_id).await;

    h.route(&h.a, &slack_result("first"), 1).await;
    egress_worker::sweep_once(&h.state).await;

    let row = &h.store.list_result_deliveries(h.a.thread_id).await.unwrap()[0];
    assert_eq!(row.status, status::DELIVERED);
    assert!(drain(&mut stream).await.is_empty());
    assert!(h.logged(&h.a).await.is_empty());
}
