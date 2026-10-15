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
/// another engine, nor one not routed at all.
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

/// A thread's task attributes, whole: setting them replaces all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskAttributes {
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<TaskEngine>,
    #[serde(default)]
    pub threat_model: bool,
}
