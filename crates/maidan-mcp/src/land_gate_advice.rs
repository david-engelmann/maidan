//! Advice at the land gate, shared with the HTTP advisor.
//!
//! The MCP tool calls this and does not write the gate. The HTTP server
//! installs the same advisor it serves on `POST /threads/{id}/land-gate/advice`.

use maidan_types::{ThreadId, WorkspaceId};
use serde_json::Value;

#[derive(Debug)]
pub enum LandGateAdviseError {
    /// The call cannot be answered as asked: the task has no result yet, or
    /// the workspace has no key to ask with.
    Invalid(String),
    NotFound,
    Unavailable(String),
}

#[async_trait::async_trait]
pub trait LandGateAdvising: Send + Sync {
    /// Advise on `thread_id`, which the caller has checked is readable by
    /// its principal and lies in `workspace_id`. `provider` names who to ask
    /// (`typesafe`, `openai` or `google`); absent, the first provider the
    /// workspace has a key for.
    async fn advise(
        &self,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        refresh: bool,
        provider: Option<String>,
    ) -> Result<Value, LandGateAdviseError>;
}
