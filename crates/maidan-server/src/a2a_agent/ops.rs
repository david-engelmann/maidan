//! The A2A operations, independent of binding. Each takes the typed request
//! and answers the typed result or an [`A2aError`]; the JSON-RPC, HTTP+JSON
//! and gRPC bindings only translate.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use maidan_a2a::page_token::{decode_task_cursor, encode_task_cursor};
use maidan_a2a::{
    is_terminal_task_state, message_content, message_parts_from_content, message_text,
    normalize_task_state, A2aError, A2aErrorKind, CancelTaskRequest, GetTaskRequest,
    ListTasksRequest, ListTasksResponse, Message as A2aMessage, Part, PartContent, Role,
    SendMessageRequest, StreamResponse, SubscribeToTaskRequest, Task, TaskStatus,
    TaskStatusUpdateEvent, TASK_STATE_CANCELED, TASK_STATE_COMPLETED, TASK_STATE_INPUT_REQUIRED,
    TASK_STATE_WORKING,
};
use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_WRITE};
use maidan_auth::{AuthContext, AuthError, ThreadScope};
use maidan_store::{A2aTaskQuery, A2aTaskWrite, PendingGateQuery, StoreError};
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, ChannelId, MessageId, NewChannel, NewMessage,
    NewThread, StrongRef, ThreadId, WorkspaceId,
};
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use super::error::{denied, hidden, internal, store};
use super::push;
use crate::state::AppState;

/// The public channel that holds conversations A2A clients start.
pub(super) const A2A_CHANNEL: &str = "a2a";
const DEFAULT_PAGE_SIZE: i32 = 50;
const MAX_PAGE_SIZE: i32 = 100;
const SUBSCRIBE_POLL: Duration = Duration::from_millis(100);
const SUBSCRIBE_MAX_POLLS: u32 = 300;

/// Now, at the millisecond precision task timestamps and page tokens carry.
fn now() -> DateTime<Utc> {
    to_millis(Utc::now())
}

fn to_millis(at: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(at.timestamp_millis()).unwrap_or(at)
}

/// The first millisecond instant at or after `at`: task and gate timestamps
/// are whole milliseconds, so "at or after `at`" is "at or after this".
fn ceil_millis(at: DateTime<Utc>) -> DateTime<Utc> {
    let floor = to_millis(at);
    if floor < at {
        floor + chrono::Duration::milliseconds(1)
    } else {
        floor
    }
}

fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn require(auth: &AuthContext, capability: &str) -> Result<(), A2aError> {
    auth.require_capability(capability).map_err(denied)
}

/// A list call's `pageSize`: unset means the default; outside `1..=100` is
/// refused.
pub(super) fn page_size(requested: Option<i32>) -> Result<i32, A2aError> {
    match requested {
        None => Ok(DEFAULT_PAGE_SIZE),
        Some(n) if (1..=MAX_PAGE_SIZE).contains(&n) => Ok(n),
        Some(n) => Err(A2aError::invalid_params(format!(
            "pageSize must be between 1 and {MAX_PAGE_SIZE}, got {n}"
        ))),
    }
}

/// `historyLength`: unset means the full history; negative is refused.
fn history_length(requested: Option<i32>) -> Result<Option<usize>, A2aError> {
    match requested {
        None => Ok(None),
        Some(n) => usize::try_from(n).map(Some).map_err(|_| {
            A2aError::invalid_params(format!("historyLength must be non-negative, got {n}"))
        }),
    }
}

// ===== Task storage and rendering =====

/// A task found by id: a stored delivery, or a pending approval gate shown as
/// an `input-required` task.
pub(super) enum Found {
    Task {
        workspace_id: WorkspaceId,
        task: Task,
    },
    Gate(ApprovalGate),
}

/// Look up a task the caller may read. Missing and inaccessible both answer
/// `TaskNotFound`.
pub(super) async fn find(
    state: &AppState,
    auth: &AuthContext,
    task_id: &str,
) -> Result<Found, A2aError> {
    if let Some(row) = state.store.get_a2a_task(task_id).await.map_err(store)? {
        let task: Task = serde_json::from_value(row.task_json).map_err(internal)?;
        auth.ensure_workspace(row.workspace_id)
            .map_err(|e| hidden(task_id, e))?;
        if let Some(thread_id) = task_thread(state, row.workspace_id, &task).await? {
            maidan_auth::ensure_thread_access(state.store.as_ref(), auth, thread_id)
                .await
                .map_err(|e| hidden(task_id, e))?;
        }
        return Ok(Found::Task {
            workspace_id: row.workspace_id,
            task,
        });
    }
    let gate = match Uuid::parse_str(task_id) {
        Ok(id) => state
            .store
            .get_approval_gate(ApprovalGateId(id))
            .await
            .map_err(store)?,
        Err(_) => None,
    };
    let Some(gate) = gate.filter(|g| g.state == ApprovalGateState::Pending) else {
        return Err(A2aError::task_not_found(task_id));
    };
    auth.ensure_workspace(gate.workspace_id)
        .map_err(|e| hidden(task_id, e))?;
    if let Some(thread_id) = gate.thread_id {
        maidan_auth::ensure_thread_access(state.store.as_ref(), auth, thread_id)
            .await
            .map_err(|e| hidden(task_id, e))?;
    }
    Ok(Found::Gate(gate))
}

/// The Maidan thread behind a task: `metadata.maidan.threadId`, else the
/// thread its context names.
async fn task_thread(
    state: &AppState,
    workspace_id: WorkspaceId,
    task: &Task,
) -> Result<Option<ThreadId>, A2aError> {
    let recorded = task
        .metadata
        .as_ref()
        .and_then(|m| m.pointer("/maidan/threadId"))
        .and_then(Value::as_str)
        .and_then(|s| Uuid::parse_str(s).ok());
    if let Some(id) = recorded {
        return Ok(Some(ThreadId(id)));
    }
    match &task.context_id {
        Some(context_id) => context_thread(state, workspace_id, context_id).await,
        None => Ok(None),
    }
}

/// The thread a context names: a bound client context, else a thread id.
async fn context_thread(
    state: &AppState,
    workspace_id: WorkspaceId,
    context_id: &str,
) -> Result<Option<ThreadId>, A2aError> {
    if let Some(thread) = state
        .store
        .get_a2a_context_thread(workspace_id, context_id)
        .await
        .map_err(store)?
    {
        return Ok(Some(thread));
    }
    Ok(Uuid::parse_str(context_id).ok().map(ThreadId))
}

/// Persist a task. The row holds no words: history and status messages are
/// dropped and rendered from the message log on read.
pub(super) async fn save(
    state: &AppState,
    workspace_id: WorkspaceId,
    task: &Task,
    status_at: DateTime<Utc>,
) -> Result<(), A2aError> {
    let mut row = task.clone();
    row.history = None;
    row.artifacts = None;
    row.status.message = None;
    let task_json = serde_json::to_value(&row).map_err(internal)?;
    state
        .store
        .upsert_a2a_task(A2aTaskWrite {
            workspace_id,
            task_id: &task.id,
            context_id: task.context_id.as_deref(),
            state: &task.status.state,
            status_at,
            task_json,
        })
        .await
        .map_err(store)
}

fn source_message_id(task: &Task) -> Option<MessageId> {
    task.metadata
        .as_ref()?
        .pointer("/maidan/messageId")?
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok())
        .map(MessageId)
}

/// A stored task as the protocol shows it: history rendered from the sealed
/// message log (a shredded message is simply absent) and cut to
/// `history_length`.
pub(super) async fn render(
    state: &AppState,
    mut task: Task,
    history_length: Option<usize>,
) -> Result<Task, A2aError> {
    task.history = None;
    if history_length == Some(0) {
        return Ok(task);
    }
    let Some(message_id) = source_message_id(&task) else {
        return Ok(task);
    };
    let stored = match state.store.get_message(message_id).await {
        Ok(message) => Some(message),
        Err(StoreError::NotFound) => None,
        Err(err) => return Err(store(err)),
    };
    let history: Vec<A2aMessage> = stored
        .and_then(|m| history_message(&m, &task))
        .into_iter()
        .collect();
    task.history = cut(history, history_length);
    Ok(task)
}

/// The last `history_length` messages, or `None` when there are none.
fn cut(mut history: Vec<A2aMessage>, history_length: Option<usize>) -> Option<Vec<A2aMessage>> {
    if let Some(n) = history_length {
        history.drain(..history.len().saturating_sub(n));
    }
    (!history.is_empty()).then_some(history)
}

/// The caller's message, rebuilt from what Maidan stored.
fn history_message(stored: &maidan_types::Message, task: &Task) -> Option<A2aMessage> {
    if stored.tombstoned_at.is_some() {
        return None;
    }
    let a2a = stored.metadata.get("a2a");
    let field = |name: &str| a2a.and_then(|v| v.get(name)).cloned();
    let strings = |name: &str| -> Vec<String> {
        field(name)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    };
    let mut parts = message_parts_from_content(stored.content.as_deref().unwrap_or_default());
    if parts.is_empty() {
        parts.push(Part::text(stored.body.clone()));
    }
    Some(A2aMessage {
        message_id: field("messageId")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| stored.id.0.to_string()),
        context_id: task.context_id.clone(),
        task_id: Some(task.id.clone()),
        role: Role::User,
        parts,
        metadata: field("metadata"),
        extensions: strings("extensions"),
        reference_task_ids: strings("referenceTaskIds"),
    })
}

/// A pending approval gate as a synthetic `input-required` task. Its id is
/// the gate id, its context the gate's thread; the prompt is the status
/// message. A resolved gate stops appearing.
pub(super) fn gate_as_task(gate: &ApprovalGate) -> Task {
    let id = gate.id.0.to_string();
    let context_id = gate.thread_id.map(|t| t.0.to_string());
    Task {
        id: id.clone(),
        context_id: context_id.clone(),
        status: TaskStatus {
            state: TASK_STATE_INPUT_REQUIRED.to_string(),
            message: Some(A2aMessage {
                message_id: id.clone(),
                context_id,
                task_id: Some(id),
                role: Role::Agent,
                parts: vec![Part::text(gate.prompt.clone())],
                metadata: None,
                extensions: vec![],
                reference_task_ids: vec![],
            }),
            timestamp: Some(timestamp(to_millis(gate.created_at))),
        },
        artifacts: None,
        history: None,
        metadata: Some(json!({
            "maidan": { "approvalGateId": gate.id.0, "kind": "approval_gate" }
        })),
    }
}

// ===== SendMessage =====

/// `SendMessage`: post the caller's message into the thread its context
/// names and answer the completed delivery task.
///
/// The author is always the token's member. A message without `contextId`
/// starts a thread in the workspace's `a2a` channel, whose id becomes the
/// context. A `contextId` naming a readable thread posts there; any other
/// value is a client-chosen context, bound on first use to a new thread.
/// Maidan tasks complete on delivery, so a message naming an existing
/// `taskId` is refused.
pub(crate) async fn send_message(
    state: &AppState,
    auth: &AuthContext,
    req: SendMessageRequest,
) -> Result<Task, A2aError> {
    if auth.bypass {
        return Err(A2aError::new(
            A2aErrorKind::PermissionDenied,
            "SendMessage needs an authenticated member to author the message",
        ));
    }
    require(auth, MESSAGE_POST)?;
    let config = req.configuration.unwrap_or_default();
    let history_length = history_length(config.history_length)?;
    let message = req.message;
    validate_message(&message)?;
    let citations = citations(&message)?;
    let push = match config.task_push_notification_config {
        Some(config) => Some(push::prepare(state, auth, config)?),
        None => None,
    };
    if let Some(task_id) = message.task_id.as_deref().filter(|t| !t.is_empty()) {
        return Err(continue_task(state, auth, task_id, message.context_id.as_deref()).await);
    }
    let (scope, context_id) = match message.context_id.as_deref().filter(|c| !c.is_empty()) {
        Some(context_id) => (
            client_context(state, auth, context_id).await?,
            context_id.to_string(),
        ),
        None => {
            let scope = new_thread(state, auth).await?;
            (scope, scope.thread_id.0.to_string())
        }
    };

    let content = message_content(&message);
    let body = message_text(&message).unwrap_or_else(|| maidan_types::derive_body(&content));
    let mut metadata = json!({ "a2a": a2a_metadata(&message) });
    if let Some(citations) = citations {
        metadata["citations"] = citations;
    }
    let dm_conversation_id = state
        .store
        .dm_conversation_for_thread(scope.thread_id)
        .await
        .map_err(store)?
        .map(|d| d.id);
    let posted = state
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: scope.thread_id,
                author_id: auth.member_id,
                body,
                metadata,
                content: Some(content),
            },
            dm_conversation_id,
        )
        .await;
    let (posted, stored) = spawn_checked(state, auth, posted).await?;
    crate::routes::publish_stored(state, stored).await;
    crate::routes::publish_routed_mentions(state, scope.thread_id, scope.workspace_id, &posted)
        .await;

    let status_at = now();
    let task = Task {
        id: Uuid::now_v7().to_string(),
        context_id: Some(context_id),
        status: TaskStatus {
            state: TASK_STATE_COMPLETED.to_string(),
            message: None,
            timestamp: Some(timestamp(status_at)),
        },
        artifacts: None,
        history: None,
        metadata: Some(json!({
            "maidan": { "messageId": posted.id.0, "threadId": scope.thread_id.0 }
        })),
    };
    save(state, scope.workspace_id, &task, status_at).await?;
    if let Some(config) = push {
        push::attach(state, &task.id, config).await?;
    }
    push::notify(state, &task);
    let mut rendered = task;
    let history = history_message(&posted, &rendered).into_iter().collect();
    rendered.history = cut(history, history_length);
    Ok(rendered)
}

/// `SendStreamingMessage`: the delivery as a stream, a `working` task then its
/// completion.
pub(crate) async fn send_streaming_message(
    state: &AppState,
    auth: &AuthContext,
    req: SendMessageRequest,
) -> Result<Vec<StreamResponse>, A2aError> {
    let task = send_message(state, auth, req).await?;
    let mut working = task.clone();
    working.status.state = TASK_STATE_WORKING.to_string();
    let completed = TaskStatusUpdateEvent {
        task_id: task.id.clone(),
        context_id: task.context_id.clone().unwrap_or_default(),
        status: task.status,
        metadata: None,
    };
    Ok(vec![
        StreamResponse::Task(working),
        StreamResponse::StatusUpdate(completed),
    ])
}

fn validate_message(message: &A2aMessage) -> Result<(), A2aError> {
    if message.message_id.trim().is_empty() {
        return Err(A2aError::invalid_params("message.messageId is required"));
    }
    if message.role != Role::User {
        return Err(A2aError::invalid_params("message.role must be ROLE_USER"));
    }
    if message.parts.is_empty() {
        return Err(A2aError::invalid_params("message.parts must not be empty"));
    }
    for part in &message.parts {
        let plain = part
            .media_type
            .as_deref()
            .is_none_or(|m| m.split(';').next().unwrap_or_default().trim() == "text/plain");
        let accepted = match part.content {
            PartContent::Text(_) => plain,
            PartContent::Url(_) => true,
            PartContent::Raw(_) | PartContent::Data(_) => false,
        };
        if !accepted {
            let media_type = part.media_type.as_deref().unwrap_or(match part.content {
                PartContent::Data(_) => "application/json",
                _ => "application/octet-stream",
            });
            return Err(A2aError::new(
                A2aErrorKind::ContentTypeNotSupported,
                format!("{media_type} parts are not accepted; send text/plain text or a url part"),
            )
            .with("mediaType", media_type));
        }
    }
    Ok(())
}

/// `message.metadata.maidan.citations`: content-addressed references, each
/// pinning a non-empty uri and a `sha256:` hash.
fn citations(message: &A2aMessage) -> Result<Option<Value>, A2aError> {
    let Some(raw) = message
        .metadata
        .as_ref()
        .and_then(|m| m.pointer("/maidan/citations"))
    else {
        return Ok(None);
    };
    let list: Vec<StrongRef> = serde_json::from_value(raw.clone())
        .map_err(|e| A2aError::invalid_params(format!("invalid citations: {e}")))?;
    if list
        .iter()
        .any(|c| c.uri.is_empty() || !maidan_types::is_well_formed_hash(&c.content_hash))
    {
        return Err(A2aError::invalid_params(
            "a citation must pin a non-empty uri and a sha256:<hex> content_hash",
        ));
    }
    Ok(Some(raw.clone()))
}

/// The A2A envelope fields kept with the Maidan message, so history renders
/// what the client sent.
fn a2a_metadata(message: &A2aMessage) -> Value {
    let mut out = json!({ "messageId": message.message_id });
    if let Some(metadata) = &message.metadata {
        out["metadata"] = metadata.clone();
    }
    if !message.extensions.is_empty() {
        out["extensions"] = json!(message.extensions);
    }
    if !message.reference_task_ids.is_empty() {
        out["referenceTaskIds"] = json!(message.reference_task_ids);
    }
    out
}

/// Why a message naming an existing task is refused.
async fn continue_task(
    state: &AppState,
    auth: &AuthContext,
    task_id: &str,
    context_id: Option<&str>,
) -> A2aError {
    let (task, gate) = match find(state, auth, task_id).await {
        Ok(Found::Task { task, .. }) => (task, false),
        Ok(Found::Gate(gate)) => (gate_as_task(&gate), true),
        Err(err) => return err,
    };
    if context_id.is_some_and(|c| !c.is_empty() && task.context_id.as_deref() != Some(c)) {
        return A2aError::invalid_params("contextId does not match the task's context")
            .with("taskId", task_id);
    }
    if gate {
        return A2aError::unsupported(
            "this task is a pending approval gate; answer it through the approvals API",
        )
        .with("taskId", task_id);
    }
    A2aError::unsupported(format!(
        "task is {}; Maidan tasks complete on delivery, so send a new message in the context instead",
        task.status.state
    ))
    .with("taskId", task_id)
}

/// The thread a client's `contextId` names, binding a new one on first use.
async fn client_context(
    state: &AppState,
    auth: &AuthContext,
    context_id: &str,
) -> Result<ThreadScope, A2aError> {
    let workspace_id = auth.workspace_id;
    if let Some(thread_id) = state
        .store
        .get_a2a_context_thread(workspace_id, context_id)
        .await
        .map_err(store)?
    {
        return authorize(state, auth, thread_id).await;
    }
    if let Ok(id) = Uuid::parse_str(context_id) {
        match state.store.get_thread(ThreadId(id)).await {
            Ok(thread) => {
                let channel = state
                    .store
                    .get_channel(thread.channel_id)
                    .await
                    .map_err(store)?;
                // Another workspace's thread id is just an unknown context
                // here: nothing reveals that it exists.
                if channel.workspace_id == workspace_id {
                    return authorize(state, auth, thread.id).await;
                }
            }
            Err(StoreError::NotFound) => {}
            Err(err) => return Err(store(err)),
        }
    }
    let created = new_thread(state, auth).await?;
    let bound = state
        .store
        .bind_a2a_context(workspace_id, context_id, created.thread_id)
        .await
        .map_err(store)?;
    if bound == created.thread_id {
        return Ok(created);
    }
    authorize(state, auth, bound).await
}

async fn authorize(
    state: &AppState,
    auth: &AuthContext,
    thread_id: ThreadId,
) -> Result<ThreadScope, A2aError> {
    maidan_auth::authorize_thread(state.store.as_ref(), auth, thread_id)
        .await
        .map_err(|err| match err {
            AuthError::Store(StoreError::NotFound) => {
                A2aError::invalid_params("the context's thread no longer exists")
            }
            err => denied(err),
        })
}

/// A new thread in the workspace's `a2a` channel.
async fn new_thread(state: &AppState, auth: &AuthContext) -> Result<ThreadScope, A2aError> {
    require(auth, WORKSPACE_WRITE)?;
    let channel_id = a2a_channel(state, auth).await?;
    let created = state
        .store
        .create_thread_with_event(NewThread {
            channel_id,
            parent_thread_id: None,
            title: Some("A2A conversation".into()),
        })
        .await;
    let (thread, stored) = spawn_checked(state, auth, created).await?;
    crate::routes::publish_stored(state, stored).await;
    authorize(state, auth, thread.id).await
}

/// The workspace's `a2a` channel, created on first use.
async fn a2a_channel(state: &AppState, auth: &AuthContext) -> Result<ChannelId, A2aError> {
    let workspace_id = auth.workspace_id;
    let find = || async {
        state
            .store
            .list_channels(workspace_id)
            .await
            .map(|channels| {
                channels
                    .into_iter()
                    .find(|c| c.name == A2A_CHANNEL)
                    .map(|c| c.id)
            })
            .map_err(store)
    };
    if let Some(id) = find().await? {
        return Ok(id);
    }
    match state
        .store
        .create_channel_with_event(NewChannel {
            workspace_id,
            name: A2A_CHANNEL.into(),
            topic: Some("Conversations started by A2A clients".into()),
            private: false,
        })
        .await
    {
        Ok((channel, stored)) => {
            crate::routes::publish_stored(state, stored).await;
            Ok(channel.id)
        }
        // A concurrent request created it first.
        Err(StoreError::Conflict(_)) => find()
            .await?
            .ok_or_else(|| internal("a2a channel vanished after a create conflict")),
        Err(err) => Err(store(err)),
    }
}

/// A store write the spawn budget may refuse; a refusal is recorded on the
/// event stream like on the REST path.
async fn spawn_checked<T>(
    state: &AppState,
    auth: &AuthContext,
    result: Result<T, StoreError>,
) -> Result<T, A2aError> {
    match result {
        Ok(value) => Ok(value),
        Err(StoreError::SpawnRejected(denial)) => {
            crate::routes::publish(state, denial.denied_event(Some(auth.member_id))).await;
            Err(store(StoreError::SpawnRejected(denial)))
        }
        Err(err) => Err(store(err)),
    }
}

// ===== GetTask, CancelTask, SubscribeToTask =====

pub(crate) async fn get_task(
    state: &AppState,
    auth: &AuthContext,
    req: GetTaskRequest,
) -> Result<Task, A2aError> {
    require(auth, MESSAGE_POST)?;
    let history_length = history_length(req.history_length)?;
    match find(state, auth, &req.id).await? {
        Found::Task { task, .. } => render(state, task, history_length).await,
        Found::Gate(gate) => Ok(gate_as_task(&gate)),
    }
}

pub(crate) async fn cancel_task(
    state: &AppState,
    auth: &AuthContext,
    req: CancelTaskRequest,
) -> Result<Task, A2aError> {
    require(auth, MESSAGE_POST)?;
    let (workspace_id, mut task) = match find(state, auth, &req.id).await? {
        Found::Task { workspace_id, task } => (workspace_id, task),
        Found::Gate(_) => {
            return Err(A2aError::new(
                A2aErrorKind::TaskNotCancelable,
                "this task is a pending approval gate; answer it through the approvals API",
            )
            .with("taskId", &req.id))
        }
    };
    if is_terminal_task_state(&task.status.state) {
        return Err(A2aError::new(
            A2aErrorKind::TaskNotCancelable,
            format!("task is already {}", task.status.state),
        )
        .with("taskId", &req.id));
    }
    let status_at = now();
    task.status = TaskStatus {
        state: TASK_STATE_CANCELED.to_string(),
        message: None,
        timestamp: Some(timestamp(status_at)),
    };
    save(state, workspace_id, &task, status_at).await?;
    push::notify(state, &task);
    render(state, task, None).await
}

/// `SubscribeToTask`: the task now, then each status change until it ends.
/// A task that already ended has nothing to stream (§3.1.6).
pub(crate) async fn subscribe(
    state: &AppState,
    auth: &AuthContext,
    req: SubscribeToTaskRequest,
) -> Result<ReceiverStream<StreamResponse>, A2aError> {
    require(auth, MESSAGE_POST)?;
    let task = match find(state, auth, &req.id).await? {
        Found::Task { task, .. } => task,
        Found::Gate(_) => {
            return Err(A2aError::unsupported(
                "this task is a pending approval gate; follow it through the approvals API",
            )
            .with("taskId", &req.id))
        }
    };
    if is_terminal_task_state(&task.status.state) {
        return Err(A2aError::unsupported(format!(
            "task is already {}; there is nothing to subscribe to",
            task.status.state
        ))
        .with("taskId", &req.id));
    }
    let initial = render(state, task, None).await?;
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let state = state.clone();
    tokio::spawn(async move {
        let mut last = initial.status.state.clone();
        let task_id = initial.id.clone();
        if tx.send(StreamResponse::Task(initial)).await.is_err() {
            return;
        }
        for _ in 0..SUBSCRIBE_MAX_POLLS {
            if !wait_unless_closed(&tx, SUBSCRIBE_POLL).await {
                return;
            }
            let current = match state.store.get_a2a_task(&task_id).await {
                Ok(Some(row)) => match serde_json::from_value::<Task>(row.task_json) {
                    Ok(task) => task,
                    Err(err) => {
                        tracing::warn!(task_id, error = %err, "a2a subscribe: unreadable task");
                        return;
                    }
                },
                Ok(None) => return,
                Err(err) => {
                    tracing::warn!(task_id, error = %err, "a2a subscribe: task lookup failed");
                    return;
                }
            };
            if current.status.state == last {
                continue;
            }
            last = current.status.state.clone();
            let update = TaskStatusUpdateEvent {
                task_id: task_id.clone(),
                context_id: current.context_id.clone().unwrap_or_default(),
                status: current.status,
                metadata: None,
            };
            if tx.send(StreamResponse::StatusUpdate(update)).await.is_err()
                || is_terminal_task_state(&last)
            {
                return;
            }
        }
    });
    Ok(ReceiverStream::new(rx))
}

/// Sleep for `period`, unless the subscriber hangs up first. False when it
/// did, so an abandoned stream stops polling the store at once.
async fn wait_unless_closed<T>(tx: &tokio::sync::mpsc::Sender<T>, period: Duration) -> bool {
    tokio::select! {
        () = tx.closed() => false,
        () = tokio::time::sleep(period) => true,
    }
}

// ===== ListTasks =====

/// One row of a `ListTasks` page before rendering.
enum Entry {
    Stored(Task),
    Gate(Task),
}

/// Per-thread read access, memoized for one listing.
struct Access<'a> {
    state: &'a AppState,
    auth: &'a AuthContext,
    threads: HashMap<ThreadId, bool>,
}

impl<'a> Access<'a> {
    fn new(state: &'a AppState, auth: &'a AuthContext) -> Self {
        Self {
            state,
            auth,
            threads: HashMap::new(),
        }
    }

    async fn thread(&mut self, thread_id: Option<ThreadId>) -> Result<bool, A2aError> {
        let Some(thread_id) = thread_id else {
            return Ok(true);
        };
        if let Some(allowed) = self.threads.get(&thread_id) {
            return Ok(*allowed);
        }
        let allowed =
            match maidan_auth::can_access_thread(self.state.store.as_ref(), self.auth, thread_id)
                .await
            {
                Ok(allowed) => allowed,
                Err(AuthError::Store(StoreError::NotFound)) => false,
                Err(err) => return Err(denied(err)),
            };
        self.threads.insert(thread_id, allowed);
        Ok(allowed)
    }
}

/// `ListTasks`: the caller's readable tasks in its workspace, newest status
/// first, with pending approval gates merged in as `input-required` tasks.
/// Keyset-paged: `nextPageToken` encodes the last task's position.
pub(crate) async fn list_tasks(
    state: &AppState,
    auth: &AuthContext,
    req: ListTasksRequest,
) -> Result<ListTasksResponse, A2aError> {
    require(auth, MESSAGE_POST)?;
    let page_size = page_size(req.page_size)?;
    let history_length = history_length(req.history_length)?;
    let status = match req.status.as_deref().filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => Some(
            normalize_task_state(s)
                .ok_or_else(|| A2aError::invalid_params(format!("invalid status filter: {s}")))?,
        ),
    };
    let since = match req
        .status_timestamp_after
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(s) => Some(ceil_millis(
            DateTime::parse_from_rfc3339(s)
                .map_err(|_| {
                    A2aError::invalid_params(format!(
                        "statusTimestampAfter must be an RFC 3339 timestamp, got {s}"
                    ))
                })?
                .with_timezone(&Utc),
        )),
    };
    let cursor = match req.page_token.as_deref().filter(|t| !t.is_empty()) {
        None => None,
        Some(token) => Some(decode_task_cursor(token)?),
    };
    let context_id = req.context_id.as_deref().filter(|c| !c.is_empty());
    let workspace_id = auth.workspace_id;
    let filter = || A2aTaskQuery {
        context_id,
        state: status,
        updated_since: since,
        before: None,
        limit: 0,
    };
    let mut access = Access::new(state, auth);
    let want = page_size as usize + 1;

    // Stored tasks, fetched in batches until a page (plus one, to know
    // whether another page follows) of readable ones is in hand.
    let mut entries: Vec<(DateTime<Utc>, String, Entry)> = Vec::new();
    let mut before = cursor.clone();
    loop {
        let rows = state
            .store
            .list_a2a_tasks(
                workspace_id,
                A2aTaskQuery {
                    before: before.as_ref().map(|(at, id)| (*at, id.as_str())),
                    limit: want as i64,
                    ..filter()
                },
            )
            .await
            .map_err(store)?;
        let exhausted = rows.len() < want;
        for row in rows {
            before = Some((row.updated_at, row.id.clone()));
            let task: Task = match serde_json::from_value(row.task_json) {
                Ok(task) => task,
                Err(err) => {
                    tracing::warn!(task_id = row.id, error = %err, "a2a list: unreadable task");
                    continue;
                }
            };
            let thread = task_thread(state, workspace_id, &task).await?;
            if access.thread(thread).await? {
                entries.push((to_millis(row.updated_at), row.id, Entry::Stored(task)));
            }
        }
        if exhausted || entries.len() >= want {
            break;
        }
    }

    // Pending gates, fetched the same way. A gate's context is its thread,
    // so a context that is not a thread id matches no gate.
    let gate_thread = match context_id {
        None => Some(None),
        Some(context) => Uuid::parse_str(context)
            .ok()
            .filter(|id| id.to_string() == context)
            .map(|id| Some(ThreadId(id))),
    };
    let gate_filter = |thread_id| PendingGateQuery {
        thread_id,
        created_since: since,
        before: None,
        limit: 0,
    };
    let mut gates_total = 0;
    if let Some(thread_id) =
        gate_thread.filter(|_| status.is_none_or(|s| s == TASK_STATE_INPUT_REQUIRED))
    {
        // Gates in the cursor's millisecond sort on either side of it by id,
        // so the first batch starts after that millisecond and the cursor
        // itself decides.
        let mut before = cursor
            .as_ref()
            .map(|(at, _)| (*at + chrono::Duration::milliseconds(1), None));
        let mut gates_listed = 0;
        loop {
            let gates = state
                .store
                .page_pending_approval_gates(
                    workspace_id,
                    PendingGateQuery {
                        before,
                        limit: want as i64,
                        ..gate_filter(thread_id)
                    },
                )
                .await
                .map_err(store)?;
            let exhausted = gates.len() < want;
            for gate in gates {
                before = Some((gate.created_at, Some(gate.id)));
                let at = to_millis(gate.created_at);
                let id = gate.id.0.to_string();
                if cursor
                    .as_ref()
                    .is_some_and(|(c_at, c_id)| (at, id.as_str()) >= (*c_at, c_id.as_str()))
                    || !access.thread(gate.thread_id).await?
                {
                    continue;
                }
                gates_listed += 1;
                entries.push((at, id, Entry::Gate(gate_as_task(&gate))));
            }
            if exhausted || gates_listed >= want {
                break;
            }
        }
        for (thread, count) in state
            .store
            .count_pending_approval_gates_by_thread(workspace_id, gate_filter(thread_id))
            .await
            .map_err(store)?
        {
            if access.thread(thread).await? {
                gates_total += count;
            }
        }
    }

    entries.sort_by(|a, b| (b.0, &b.1).cmp(&(a.0, &a.1)));
    let next_page_token = if entries.len() > page_size as usize {
        entries.truncate(page_size as usize);
        entries
            .last()
            .map(|(at, id, _)| encode_task_cursor(&(*at, id.clone())))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let mut total_size: i64 = gates_total;
    for (context, count) in state
        .store
        .count_a2a_tasks_by_context(workspace_id, filter())
        .await
        .map_err(store)?
    {
        let thread = match context {
            Some(context) => context_thread(state, workspace_id, &context).await?,
            None => None,
        };
        if access.thread(thread).await? {
            total_size += count;
        }
    }

    let mut tasks = Vec::with_capacity(entries.len());
    for (_, _, entry) in entries {
        let mut task = match entry {
            Entry::Stored(task) => render(state, task, history_length).await?,
            Entry::Gate(task) => task,
        };
        task.artifacts = (req.include_artifacts == Some(true)).then(Vec::new);
        tasks.push(task);
    }
    Ok(ListTasksResponse {
        tasks,
        next_page_token,
        page_size,
        total_size: i32::try_from(total_size).unwrap_or(i32::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_subscriber_hanging_up_ends_the_wait_at_once() {
        let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
        assert!(wait_unless_closed(&tx, Duration::from_millis(1)).await);
        drop(rx);
        let waited = tokio::time::timeout(
            Duration::from_secs(5),
            wait_unless_closed(&tx, Duration::from_secs(3600)),
        )
        .await;
        assert_eq!(waited, Ok(false));
    }

    #[test]
    fn history_length_must_be_non_negative() {
        assert_eq!(history_length(None).unwrap(), None);
        assert_eq!(history_length(Some(0)).unwrap(), Some(0));
        assert_eq!(
            history_length(Some(-1)).unwrap_err().kind,
            A2aErrorKind::InvalidParams
        );
    }

    fn message(parts: Vec<Part>) -> A2aMessage {
        A2aMessage {
            message_id: "m1".into(),
            context_id: None,
            task_id: None,
            role: Role::User,
            parts,
            metadata: None,
            extensions: vec![],
            reference_task_ids: vec![],
        }
    }

    #[test]
    fn only_plain_text_and_url_parts_are_accepted() {
        let url = Part {
            content: PartContent::Url("https://example.com/a.pdf".into()),
            metadata: None,
            filename: None,
            media_type: Some("application/pdf".into()),
        };
        assert!(validate_message(&message(vec![Part::text("hi"), url])).is_ok());
        let mut plain = Part::text("hi");
        plain.media_type = Some("text/plain; charset=utf-8".into());
        assert!(validate_message(&message(vec![plain])).is_ok());

        let mut markdown = Part::text("# hi");
        markdown.media_type = Some("text/markdown".into());
        let data = Part {
            content: PartContent::Data(json!({ "k": 1 })),
            metadata: None,
            filename: None,
            media_type: None,
        };
        for part in [markdown, data] {
            let err = validate_message(&message(vec![part])).unwrap_err();
            assert_eq!(err.kind, A2aErrorKind::ContentTypeNotSupported);
        }
    }

    #[test]
    fn a_message_needs_an_id_a_user_role_and_parts() {
        let mut m = message(vec![]);
        assert_eq!(
            validate_message(&m).unwrap_err().kind,
            A2aErrorKind::InvalidParams
        );
        m.parts.push(Part::text("hi"));
        m.message_id = " ".into();
        assert!(validate_message(&m).is_err());
        m.message_id = "m1".into();
        m.role = Role::Agent;
        assert!(validate_message(&m).is_err());
    }

    #[test]
    fn citations_must_pin_a_hash() {
        let mut m = message(vec![Part::text("hi")]);
        assert_eq!(citations(&m).unwrap(), None);
        m.metadata = Some(json!({ "maidan": { "citations": [
            { "uri": "maidan:event/9", "content_hash": format!("sha256:{}", "a".repeat(64)) }
        ] } }));
        assert!(citations(&m).unwrap().is_some());
        m.metadata = Some(json!({ "maidan": { "citations": [
            { "uri": "maidan:event/9", "content_hash": "nope" }
        ] } }));
        assert_eq!(citations(&m).unwrap_err().kind, A2aErrorKind::InvalidParams);
    }
}
