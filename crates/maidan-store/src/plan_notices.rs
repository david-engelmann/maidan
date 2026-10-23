//! The system messages a plan's follow-ups post (#1421), shared by both
//! backends so the wording and the `notice` names never drift apart.

use maidan_types::ThreadId;
use serde_json::{json, Value};

/// A child as the parent's messages name it.
pub(crate) struct ChildLine {
    pub thread_id: ThreadId,
    pub title: String,
    /// The landed pull request, when there is one.
    pub landed: Option<(String, i64)>,
}

/// On the parent, when a child closes without landing.
pub(crate) fn unlanded_body(child: &ChildLine) -> String {
    format!(
        "Plan child \"{}\" ({}) closed without landing, so this plan stays open and the \
         children that depend on it are blocked. Clear their blocks once the work is redone, \
         or plan again.",
        child.title, child.thread_id.0
    )
}

pub(crate) fn unlanded_metadata(child: ThreadId) -> Value {
    json!({ "notice": "plan_child_unlanded", "child_thread_id": child.0 })
}

/// The block note on a dependent of a child that closed without landing.
pub(crate) fn blocked_note(dependency_title: &str, dependency: ThreadId) -> String {
    format!(
        "depends on \"{dependency_title}\" ({}), which closed without landing",
        dependency.0
    )
}

/// On that dependent, so its worker sees why the queue skips it.
pub(crate) fn blocked_body(dependency_title: &str, dependency: ThreadId) -> String {
    format!(
        "Blocked: this {}. Starting now would build on a base that never merged. A person \
         or the planner clears the block once the dependency is redone.",
        blocked_note(dependency_title, dependency)
    )
}

pub(crate) fn blocked_metadata(dependency: ThreadId) -> Value {
    json!({ "notice": "dependency_unlanded", "depends_on_thread_id": dependency.0 })
}

/// On the parent, when its last child lands and it closes.
pub(crate) fn landed_body(children: &[ChildLine]) -> String {
    let mut body = format!("Every child of this plan landed ({}):", children.len());
    for child in children {
        match &child.landed {
            Some((repo, pr)) => body.push_str(&format!("\n- {}: {repo}#{pr}", child.title)),
            None => body.push_str(&format!("\n- {}", child.title)),
        }
    }
    body
}

pub(crate) fn landed_metadata() -> Value {
    json!({ "notice": "plan_landed" })
}

/// On the parent, when every child landed but a close gate refused it.
pub(crate) fn refused_body(reason: &str) -> String {
    format!(
        "Every child of this plan landed, but the plan could not close: {reason}. It stays \
         open until that is resolved and someone closes it."
    )
}

pub(crate) fn refused_metadata(reason: &str) -> Value {
    json!({ "notice": "plan_close_refused", "reason": reason })
}

/// How a closed child landed, under the hybrid signal the planner
/// amendment specified: a `ThreadLanded` event or a merged change pull
/// names the pull request, and a plain close with no change pull at all
/// lands without one. A change pull that closed unmerged is the one
/// close that is not a landing.
pub(crate) enum Landing {
    Pull(String, i64),
    Plain,
}

impl Landing {
    /// The pull request the child landed with, when there is one.
    pub(crate) fn pull(&self) -> Option<(String, i64)> {
        match self {
            Landing::Pull(repo, pr) => Some((repo.clone(), *pr)),
            Landing::Plain => None,
        }
    }
}

/// The landing a `ThreadLanded` payload records, when it is one.
pub(crate) fn landing_from_payload(payload: &Value) -> Option<(String, i64)> {
    let repo = payload.get("repo")?.as_str()?.to_string();
    let pr = payload.get("pr_number")?.as_i64()?;
    Some((repo, pr))
}
