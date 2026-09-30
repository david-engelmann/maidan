//! Response models, from the server's OpenAPI schemas.
//!
//! Every model keeps the members it does not declare (added to the server after
//! this crate was published) in `extra`, so a new field never fails a response;
//! [`Workspace::unknown_members`] and its twins list them. Timestamps are RFC
//! 3339 strings. A string enum carries an `Other` variant for a value this crate
//! does not know, so a new state or kind decodes too.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

/// Collects the paths of members a model did not declare, recursively.
trait Members {
    fn unknown(&self, _path: &str, _out: &mut Vec<String>) {}
}

impl Members for String {}
impl Members for bool {}
impl Members for i64 {}
impl Members for Value {}

impl<T: Members> Members for Option<T> {
    fn unknown(&self, path: &str, out: &mut Vec<String>) {
        if let Some(v) = self {
            v.unknown(path, out);
        }
    }
}

impl<T: Members> Members for Vec<T> {
    fn unknown(&self, path: &str, out: &mut Vec<String>) {
        for (i, v) in self.iter().enumerate() {
            v.unknown(&format!("{path}[{i}]"), out);
        }
    }
}

macro_rules! open_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $wire:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
            /// A value this crate does not know yet.
            Other(String),
        }

        impl $name {
            /// The wire form.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $wire,)+
                    Self::Other(s) => s,
                }
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                match s {
                    $($wire => Self::$variant,)+
                    other => Self::Other(other.to_string()),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                Ok(String::deserialize(d)?.as_str().into())
            }
        }

        impl Members for $name {}
    };
}

macro_rules! model {
    ($(#[$meta:meta])* $name:ident { $($(#[$fmeta:meta])* $field:ident : $ty:ty),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        #[non_exhaustive]
        pub struct $name {
            $($(#[$fmeta])* pub $field: $ty,)+
            /// Members the server sent that this model does not declare.
            #[serde(flatten)]
            pub extra: Map<String, Value>,
        }

        impl $name {
            /// Paths of every member, here or nested, that the server sent and
            /// this crate does not model. Empty when the models match the server.
            pub fn unknown_members(&self) -> Vec<String> {
                let mut out = Vec::new();
                Members::unknown(self, stringify!($name), &mut out);
                out
            }
        }

        impl Members for $name {
            fn unknown(&self, path: &str, out: &mut Vec<String>) {
                out.extend(self.extra.keys().map(|k| format!("{path}.{k}")));
                $(Members::unknown(&self.$field, &format!("{path}.{}", stringify!($field)), out);)+
            }
        }
    };
}

open_enum! {
    /// A thread's FSM state.
    ThreadState { Open => "open", InReview => "in_review", Closed => "closed", Archived => "archived" }
}

open_enum! {
    /// Whether a member is a person or an agent.
    MemberKind { Human => "human", Agent => "agent" }
}

open_enum! {
    /// What an uploaded artifact is.
    ArtifactKind {
        Screenshot => "screenshot",
        Recording => "recording",
        Transcript => "transcript",
        CodeDump => "code_dump",
        Attachment => "attachment",
        ContextSnapshot => "context_snapshot",
    }
}

open_enum! {
    /// How an import treats ids: `New` remaps them, `Restore` keeps them.
    ImportMode { New => "new", Restore => "restore" }
}

open_enum! {
    /// One end of a [`Reference`].
    RefSide { Thread => "thread", Message => "message" }
}

open_enum! {
    /// A reviewer's verdict.
    ReviewDecision { Approve => "approve", RequestChanges => "request_changes" }
}

model! {
    Workspace {
        id: String,
        name: String,
        created_at: String,
        updated_at: String,
        tombstoned_at: Option<String>,
    }
}

model! {
    ImportResult {
        workspace_id: String,
        mode: ImportMode,
    }
}

model! {
    Member {
        id: String,
        workspace_id: String,
        handle: String,
        kind: MemberKind,
        display_name: Option<String>,
        created_at: String,
        updated_at: String,
        tombstoned_at: Option<String>,
    }
}

model! {
    TokenQuota {
        capability: String,
        max_per_window: i64,
        window_secs: i64,
    }
}

model! {
    /// A mint's answer. `secret` is returned here once and never again.
    MintedToken {
        id: String,
        secret: String,
        workspace_id: String,
        member_id: String,
        capabilities: Vec<String>,
        expires_at: Option<String>,
        quotas: Vec<TokenQuota>,
    }
}

model! {
    /// Token metadata; it never carries the secret.
    TokenSummary {
        id: String,
        workspace_id: String,
        member_id: String,
        label: Option<String>,
        capabilities: Vec<String>,
        created_at: String,
        expires_at: Option<String>,
        revoked_at: Option<String>,
    }
}

model! {
    Channel {
        id: String,
        workspace_id: String,
        name: String,
        private: bool,
        topic: Option<String>,
        created_at: String,
        updated_at: String,
        tombstoned_at: Option<String>,
    }
}

model! {
    Thread {
        id: String,
        channel_id: String,
        parent_thread_id: Option<String>,
        title: Option<String>,
        state: ThreadState,
        assignee_id: Option<String>,
        owner_id: Option<String>,
        assignment_expires_at: Option<String>,
        /// The fencing token [`crate::Client::renew_claim`] takes.
        claim_lease_id: Option<String>,
        work_started_at: Option<String>,
        created_at: String,
        updated_at: String,
        tombstoned_at: Option<String>,
    }
}

model! {
    /// A content-addressed pin (`maidan:event/{id}` plus its hash).
    StrongRef {
        uri: String,
        content_hash: String,
    }
}

/// A claim: the thread's fields at the top level, plus the pin. Derefs to the
/// [`Thread`], so `claim.id` reads the thread's id.
///
/// Hand-written rather than `model!`: a second flattened map beside `thread`
/// would receive every thread member too. Unknown members land in
/// `thread.extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClaimedThread {
    #[serde(flatten)]
    pub thread: Thread,
    pub pin: StrongRef,
}

impl ClaimedThread {
    /// See [`Workspace::unknown_members`].
    pub fn unknown_members(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.thread.unknown("ClaimedThread", &mut out);
        self.pin.unknown("ClaimedThread.pin", &mut out);
        out
    }
}

impl std::ops::Deref for ClaimedThread {
    type Target = Thread;
    fn deref(&self) -> &Thread {
        &self.thread
    }
}

model! {
    ThreadResult {
        thread_id: String,
        /// The producer's JSON, as it was set.
        result: Value,
        produced_by: String,
        produced_at: String,
    }
}

model! {
    /// A structured message block, discriminated by `block_type` (`text`,
    /// `code`, `tool_use`, `tool_result`, `resource_link`); each type sets its
    /// own fields.
    ContentBlock {
        #[serde(rename = "type")]
        block_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_use_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    }
}

model! {
    Message {
        id: String,
        thread_id: String,
        author_id: String,
        body: String,
        content: Option<Vec<ContentBlock>>,
        metadata: Option<Value>,
        posted_at: String,
        edited_at: Option<String>,
        tombstoned_at: Option<String>,
    }
}

model! {
    Artifact {
        id: String,
        sha256: String,
        size_bytes: i64,
        kind: ArtifactKind,
        mime_type: Option<String>,
        uploaded_by: Option<String>,
        created_at: String,
        tombstoned_at: Option<String>,
    }
}

model! {
    MessageEditView {
        id: i64,
        message_id: String,
        editor_id: String,
        edited_at: String,
        /// Sent only with `include_edits=true`.
        body_before: Option<String>,
        body_after: Option<String>,
    }
}

model! {
    Reference {
        id: String,
        src_kind: RefSide,
        src_id: String,
        dst_kind: RefSide,
        dst_id: String,
        relation: String,
        created_at: String,
    }
}

model! {
    ThreadTransition {
        id: String,
        thread_id: String,
        from_state: ThreadState,
        to_state: ThreadState,
        actor_id: String,
        occurred_at: String,
    }
}

model! {
    ThreadFsmContext {
        state: ThreadState,
        transitions: Vec<ThreadTransition>,
    }
}

model! {
    AcceptedDecision {
        thread_id: String,
        state: ThreadState,
        title: Option<String>,
        produced_by: String,
        produced_at: String,
        result_kind: Option<String>,
        status: Option<String>,
        summary: Option<String>,
    }
}

model! {
    ThreadReview {
        thread_id: String,
        reviewer_id: String,
        actor_id: Option<String>,
        decision: ReviewDecision,
        note: Option<String>,
        dismissed_at: Option<String>,
        created_at: String,
        updated_at: String,
    }
}

model! {
    GlossaryTerm {
        id: String,
        workspace_id: String,
        term: String,
        definition: String,
        aliases: Vec<String>,
        created_by: String,
        created_at: String,
        updated_at: String,
    }
}

model! {
    PackElision {
        elided_message_count: i64,
        elided_token_estimate: i64,
        first_elided_id: String,
        last_elided_id: String,
        summary: String,
    }
}

model! {
    ParentGrounding {
        thread_id: String,
        state: ThreadState,
        title: Option<String>,
        opening_message: Option<Message>,
        latest_result: Option<Value>,
    }
}

model! {
    /// `GET /threads/{id}/context`: the context pack a claimer reads.
    ThreadContext {
        workspace_id: String,
        channel_id: String,
        thread: Thread,
        messages: Vec<Message>,
        message_edits: Vec<MessageEditView>,
        references: Vec<Reference>,
        artifacts: Vec<Artifact>,
        fsm: ThreadFsmContext,
        #[serde(default)]
        accepted_decisions: Vec<AcceptedDecision>,
        #[serde(default)]
        change_requests: Vec<ThreadReview>,
        #[serde(default)]
        glossary: Vec<GlossaryTerm>,
        elision: Option<PackElision>,
        parent_grounding: Option<ParentGrounding>,
        next_message_cursor: Option<String>,
    }
}

model! {
    /// A row of `GET /workspaces/{id}/events`. `payload` is the event itself,
    /// shaped by `kind`.
    StoredEvent {
        #[serde(rename = "$type")]
        event_type: String,
        id: i64,
        lsn: i64,
        kind: String,
        workspace_id: Option<String>,
        channel_id: Option<String>,
        thread_id: Option<String>,
        payload: Value,
        occurred_at: String,
        prev_hash: String,
        content_hash: String,
        content_key: Option<String>,
        traceparent: Option<String>,
        tracestate: Option<String>,
    }
}
