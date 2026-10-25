//! The hard allowance cutoff on each background meter: an open sign-up
//! workspace past its allowance gets no deliveries (egress, web push, mail),
//! runs no WASI handler, while an invited or operator-created workspace past
//! the same allowance is only marked and keeps working. A new workspace can
//! still sign up while another is cut off.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::github::{route_message_to_github, GithubError, GithubSender};
use maidan_server::web_push::{WebPushError, WebPushSender};
use maidan_server::{egress_worker, mail_worker, notification_router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::*;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

const METERED: [UsageMeter; 4] = [
    UsageMeter::Deliveries,
    UsageMeter::WasiMs,
    UsageMeter::EmbeddingCalls,
    UsageMeter::StoredBytes,
];

async fn setup() -> (AppState, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
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
    std::mem::forget(dir);
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    state.usage_allowance = Arc::new(
        UsageAllowance::parse("deliveries=10,wasi_ms=10,embedding_calls=10,stored_bytes=10")
            .unwrap(),
    );
    (state, store)
}

async fn signed_up(store: &dyn Store, who: &str, open: bool) -> (WorkspaceId, MemberId) {
    let outcome = store
        .complete_oidc_signup(
            NewOidcSignup {
                workspace_name: format!("{who}'s workspace"),
                handle: who.into(),
                issuer: "https://idp.example".into(),
                subject: format!("sub-{who}"),
                email: format!("{who}@example.com"),
                invite_entry_id: None,
                session_expires_at: Utc::now() + Duration::from_secs(3600),
                open,
            },
            Box::new(|core| NewAuditEvent {
                scope: AuditScope::Workspace(core.workspace.id),
                actor_id: Some(core.member.id),
                action: "workspace.signup".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(core.workspace.id.0),
                metadata: json!({}),
            }),
            Box::new(|session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: "session.create".into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: json!({}),
            }),
        )
        .await
        .unwrap();
    (outcome.workspace.id, outcome.member.id)
}

async fn operator_workspace(store: &dyn Store) -> (WorkspaceId, MemberId) {
    let ws = store
        .create_workspace(NewWorkspace { name: "ops".into() })
        .await
        .unwrap();
    let m = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "admin".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    (ws.id, m.id)
}

/// Mark `ws` past every metered allowance this month, the way the flush
/// does, so the cutoff set is built by the real refresh.
async fn past_every_allowance(store: &dyn Store, ws: WorkspaceId) {
    let month = UsageMonth::of(Utc::now().date_naive()).label();
    for meter in METERED {
        let mark = OverAllowance {
            workspace_id: ws,
            meter,
            month: month.clone(),
            quantity: 11,
            allowance: 10,
            marked_at: Utc::now(),
        };
        store.mark_over_allowance(&mark, &[]).await.unwrap();
    }
}

struct Tenants {
    open: (WorkspaceId, MemberId),
    invited: (WorkspaceId, MemberId),
    ops: (WorkspaceId, MemberId),
}

/// An open, an invited and an operator workspace, all three past every
/// metered allowance, and the cutoff refreshed.
async fn three_past_it(state: &AppState, store: &dyn Store) -> Tenants {
    let t = Tenants {
        open: signed_up(store, "ada", true).await,
        invited: signed_up(store, "cy", false).await,
        ops: operator_workspace(store).await,
    };
    for (ws, _) in [t.open, t.invited, t.ops] {
        past_every_allowance(store, ws).await;
    }
    maidan_server::usage_meters::refresh_cut_off(state).await;
    t
}

#[tokio::test]
async fn only_the_open_workspace_is_cut_off_and_invited_and_operator_ones_are_only_marked() {
    let (state, store) = setup().await;
    let t = three_past_it(&state, store.as_ref()).await;
    for meter in METERED {
        assert!(state.usage_meters.is_cut_off(t.open.0, meter), "{meter:?}");
        assert!(
            !state.usage_meters.is_cut_off(t.invited.0, meter),
            "{meter:?}"
        );
        assert!(!state.usage_meters.is_cut_off(t.ops.0, meter), "{meter:?}");
    }
}

#[tokio::test]
async fn a_new_workspace_can_sign_up_while_another_open_one_is_cut_off() {
    let (state, store) = setup().await;
    let t = three_past_it(&state, store.as_ref()).await;
    let (fresh, _) = signed_up(store.as_ref(), "dee", true).await;
    maidan_server::usage_meters::refresh_cut_off(&state).await;
    for meter in METERED {
        assert!(!state.usage_meters.is_cut_off(fresh, meter), "{meter:?}");
        assert!(state.usage_meters.is_cut_off(t.open.0, meter), "{meter:?}");
    }
}

struct MockGithub {
    sent: Mutex<Vec<i64>>,
}

#[async_trait::async_trait]
impl GithubSender for MockGithub {
    async fn post_comment(
        &self,
        repo: &str,
        issue_number: i64,
        _text: &str,
    ) -> Result<Option<ExternalRef>, GithubError> {
        self.sent.lock().unwrap().push(issue_number);
        Ok(Some(ExternalRef::Github {
            repo: repo.into(),
            comment_id: issue_number,
        }))
    }

    async fn update_comment(
        &self,
        _repo: &str,
        _comment_id: i64,
        _text: &str,
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never updates; that is result delivery")
    }

    async fn list_issue_comments(
        &self,
        _repo: &str,
        _issue_number: i64,
    ) -> Result<Vec<maidan_server::github::GithubIssueComment>, GithubError> {
        unreachable!("the projector egress never lists comments; that is result delivery")
    }

    async fn create_check_run(
        &self,
        _repo: &str,
        _check: &maidan_types::GithubCheckRun,
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a check run; that is result delivery")
    }

    async fn create_review(
        &self,
        _repo: &str,
        _pull_number: i64,
        _commit_id: &str,
        _comments: &[maidan_types::GithubReviewComment],
    ) -> Result<(), GithubError> {
        unreachable!("the projector egress never creates a review; that is result delivery")
    }
}

async fn linked_message(
    state: &AppState,
    store: &dyn Store,
    (ws, member): (WorkspaceId, MemberId),
    issue: i64,
) {
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws,
            name: "eng".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    store
        .link_github_issue(NewGithubIssueLink {
            repo: "o/r".into(),
            issue_number: issue,
            workspace_id: ws,
            channel_id: channel.id,
            thread_id: thread.id,
            member_id: member,
        })
        .await
        .unwrap();
    let (m, _) = store
        .post_message_with_event(
            NewMessage {
                thread_id: thread.id,
                author_id: member,
                body: "shipping it".into(),
                metadata: json!({}),
                content: None,
            },
            None,
        )
        .await
        .unwrap();
    route_message_to_github(state, issue, thread.id, &m).await;
}

#[tokio::test]
async fn egress_for_a_cut_off_workspace_is_parked_while_other_tenants_are_delivered() {
    let (mut state, store) = setup().await;
    let github = Arc::new(MockGithub {
        sent: Mutex::new(Vec::new()),
    });
    state.attach_github_sender(github.clone());
    let t = three_past_it(&state, store.as_ref()).await;
    linked_message(&state, store.as_ref(), t.open, 1).await;
    linked_message(&state, store.as_ref(), t.invited, 2).await;
    linked_message(&state, store.as_ref(), t.ops, 3).await;

    let stats = egress_worker::sweep_once(&state).await;
    assert_eq!(stats.sent, 2, "{stats:?}");
    assert_eq!(stats.deferred, 1, "{stats:?}");
    assert_eq!(stats.dead, 0, "parked, not failed: {stats:?}");
    let mut sent = github.sent.lock().unwrap().clone();
    sent.sort();
    assert_eq!(sent, vec![2, 3]);

    // Parked for a while, not retried in a loop: a second sweep sends nothing.
    let again = egress_worker::sweep_once(&state).await;
    assert_eq!(again.sent + again.deferred + again.dead, 0, "{again:?}");
    // And it is still queued for after the cutoff lifts.
    let later = Utc::now() + chrono::Duration::minutes(16);
    let parked = store.claim_next_due_egress(later, 30).await.unwrap();
    assert_eq!(parked.map(|e| e.workspace_id), Some(t.open.0));
}

struct RecordingPush {
    sent: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WebPushSender for RecordingPush {
    async fn send(&self, sub: &PushSubscription, _payload: &[u8]) -> Result<(), WebPushError> {
        self.sent.lock().unwrap().push(sub.endpoint.clone());
        Ok(())
    }
}

#[tokio::test]
async fn web_push_for_a_cut_off_workspace_is_not_sent_while_other_tenants_are() {
    let (mut state, store) = setup().await;
    let push = Arc::new(RecordingPush {
        sent: Mutex::new(Vec::new()),
    });
    state.attach_web_push(push.clone());
    let t = three_past_it(&state, store.as_ref()).await;
    for (name, (_, member)) in [("open", t.open), ("invited", t.invited), ("ops", t.ops)] {
        store
            .add_push_subscription(NewPushSubscription {
                member_id: member,
                endpoint: format!("https://push.example.com/{name}"),
                p256dh: "k".into(),
                auth: "s".into(),
            })
            .await
            .unwrap();
        notification_router::deliver_notification_web_push(
            &state,
            member,
            EventKind::MentionRecorded,
            1,
        )
        .await;
    }
    let mut sent = push.sent.lock().unwrap().clone();
    sent.sort();
    assert_eq!(
        sent,
        vec![
            "https://push.example.com/invited".to_string(),
            "https://push.example.com/ops".to_string(),
        ]
    );
}

struct RecordingMailer {
    sent: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl maidan_server::mail::MailTransport for RecordingMailer {
    async fn send(
        &self,
        to: &str,
        _subject: &str,
        _body: &str,
    ) -> Result<(), maidan_server::mail::MailError> {
        self.sent.lock().unwrap().push(to.to_string());
        Ok(())
    }
}

#[tokio::test]
async fn mail_for_a_cut_off_workspace_is_parked_while_other_tenants_are_sent() {
    let (mut state, store) = setup().await;
    let mailer = Arc::new(RecordingMailer {
        sent: Mutex::new(Vec::new()),
    });
    state.attach_mail(mailer.clone());
    let t = three_past_it(&state, store.as_ref()).await;
    for (name, (ws, _)) in [("open", t.open), ("invited", t.invited), ("ops", t.ops)] {
        store
            .enqueue_mail(NewMailOutbox {
                workspace_id: Some(ws),
                source_log_id: None,
                to_address: format!("{name}@example.com"),
                subject: "hi".into(),
                body: "hello".into(),
            })
            .await
            .unwrap();
    }
    let stats = mail_worker::sweep_once(&state).await;
    assert_eq!(stats.sent, 2, "{stats:?}");
    assert_eq!(stats.deferred, 1, "{stats:?}");
    assert_eq!(stats.dead, 0, "{stats:?}");
    let mut sent = mailer.sent.lock().unwrap().clone();
    sent.sort();
    assert_eq!(sent, vec!["invited@example.com", "ops@example.com"]);
}

fn wasi_command(ws: WorkspaceId) -> SlashCommand {
    SlashCommand {
        id: SlashCommandId::new(),
        workspace_id: ws,
        name: "echo".into(),
        description: None,
        handler_kind: SlashHandlerKind::Wasi,
        handler_target: "0".repeat(64),
        enabled: true,
        created_at: Utc::now(),
        revoked_at: None,
    }
}

#[tokio::test]
async fn a_wasi_handler_for_a_cut_off_workspace_does_not_run_while_other_tenants_do() {
    let (state, store) = setup().await;
    let t = three_past_it(&state, store.as_ref()).await;
    let parsed = maidan_router::ParsedSlashCommand {
        name: "echo".into(),
        args: String::new(),
    };
    let auth = maidan_auth::AuthContext::bypass();
    let run = |(ws, member): (WorkspaceId, MemberId)| {
        let state = state.clone();
        let auth = auth.clone();
        let parsed = parsed.clone();
        async move {
            maidan_server::wasi_handler::dispatch_wasi(
                &state,
                &auth,
                &wasi_command(ws),
                &parsed,
                ws,
                ChannelId::new(),
                ThreadId::new(),
                member,
                MessageId::new(),
            )
            .await
        }
    };
    let refused = run(t.open).await;
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert_eq!(refused["error_kind"], json!("over_allowance"), "{refused}");
    assert!(
        refused["error"]
            .as_str()
            .unwrap_or_default()
            .contains("free allowance for wasi_ms"),
        "{refused}"
    );
    // The others get as far as loading their module (absent here), so they
    // fail for that reason, not the allowance.
    for tenant in [t.invited, t.ops] {
        let ran = run(tenant).await;
        assert_ne!(ran["error_kind"], json!("over_allowance"), "{ran}");
    }
}
