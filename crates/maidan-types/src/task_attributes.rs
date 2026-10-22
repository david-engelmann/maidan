//! Task attributes: what kind of work a thread is, who or what should do
//! it, and whether it needs a threat model.
//!
//! Routing and the swarm ramp metrics read these: a claimer can ask the
//! queue only for work meant for its engine, and the metrics can split a
//! workspace's pull requests by the kind and engine of the task behind
//! them. The attributes belong to the thread, are set at intake or replaced
//! later, and are stored beside it; a thread nobody ever described reads as
//! the defaults (a change, no engine, no threat model).

use serde::{Deserialize, Serialize};

/// What kind of work a task is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskKind {
    /// Work that lands a change (the default).
    #[default]
    Change,
    /// Work that plans other work; its fan-out is the planner's children.
    Plan,
    /// Work that reviews a change.
    Review,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Change => "change",
            Self::Plan => "plan",
            Self::Review => "review",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "change" => Some(Self::Change),
            "plan" => Some(Self::Plan),
            "review" => Some(Self::Review),
            _ => None,
        }
    }
}

/// Who or what a task is meant for. `None` on a thread means nobody has
/// routed it yet; an engine-filtered claim never takes a thread routed to
/// another engine, nor one not routed at all. A workspace's default engine
/// routes the threads created there without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskEngine {
    /// An automated Pi worker.
    Pi,
    /// An interactive agent session.
    Session,
    /// A person.
    Human,
}

impl TaskEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Session => "session",
            Self::Human => "human",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pi" => Some(Self::Pi),
            "session" => Some(Self::Session),
            "human" => Some(Self::Human),
            _ => None,
        }
    }
}

/// How big a task is, as its planner judged it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskSize {
    S,
    M,
    L,
}

impl TaskSize {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::S => "S",
            Self::M => "M",
            Self::L => "L",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "S" => Some(Self::S),
            "M" => Some(Self::M),
            "L" => Some(Self::L),
            _ => None,
        }
    }
}

/// How much review a change needs before it merges: Tier A takes the fast
/// path, Tier B a full review. A thread with no tier stored counts as B in
/// the metrics, while the stored value stays unset, so a later restatement
/// can tell "never said" from "B".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum TaskTier {
    A,
    B,
}

impl TaskTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "A" => Some(Self::A),
            "B" => Some(Self::B),
            _ => None,
        }
    }
}

/// A workspace's default engine: where a thread created there with no
/// engine named is routed. `None` leaves such threads unrouted, and an
/// engine-filtered claim never takes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceDefaultEngine {
    pub engine: Option<TaskEngine>,
}

/// The most characters an acceptance criterion may hold.
pub const ACCEPTANCE_MAX_CHARS: usize = 4000;

/// A thread's task attributes, whole: setting them replaces every field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskAttributes {
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<TaskEngine>,
    #[serde(default)]
    pub threat_model: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<TaskSize>,
    /// How a reviewer verifies the task: what the change must satisfy. Shown
    /// to the worker in the thread context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<String>,
    /// The Linear issue the task is for (`ENG-1234`). Where the workspace
    /// has turned claim comments on, the task's claim and release are said
    /// on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_issue: Option<String>,
    /// The change's review tier, set at intake and restated when a new head
    /// changes it. Read by the metrics only; nothing routes or gates on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<TaskTier>,
}

impl TaskAttributes {
    /// Trim the acceptance text (blank reads as none) and check its length.
    pub fn validated(mut self) -> Result<Self, String> {
        self.linear_issue = self
            .linear_issue
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty());
        if let Some(key) = &self.linear_issue {
            if !crate::is_issue_key(key) {
                return Err(format!(
                    "linear_issue `{key}` is not an issue key such as ENG-123"
                ));
            }
        }
        self.acceptance = self
            .acceptance
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        if let Some(text) = &self.acceptance {
            if text.chars().count() > ACCEPTANCE_MAX_CHARS {
                return Err(format!(
                    "acceptance is at most {ACCEPTANCE_MAX_CHARS} characters"
                ));
            }
            if text
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                return Err("acceptance has no control characters".into());
            }
        }
        Ok(self)
    }
}
