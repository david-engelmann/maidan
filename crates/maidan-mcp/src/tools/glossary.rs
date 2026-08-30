//! Shared-glossary MCP tools (Cluster 322): an agent defines and looks up a
//! workspace's canonical `term -> definition` (+ aliases) — the anti-drift pin.
//! The REST twins are also Cluster 322; `delete` stays REST-only (the 220/229
//! precedent). Workspace-scoped: `set` is `workspace:write` + `created_by` is the
//! caller; `get`/`list` are `workspace:read`. No channel/thread arg, so no
//! pre-dispatch access gate — the workspace cap is the control.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct SetGlossaryArgs {
    term: String,
    definition: String,
    #[serde(default)]
    aliases: Option<Vec<String>>,
}

/// Define (or redefine) a term in the caller's workspace glossary (Cluster 322).
/// Upserts on `(workspace, term)`; owned by the caller.
pub(super) async fn set_glossary_term(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetGlossaryArgs = serde_json::from_value(args.clone())?;
    if a.term.trim().is_empty() {
        return Err(McpError::InvalidParams("term must not be empty".into()));
    }
    if a.definition.trim().is_empty() {
        return Err(McpError::InvalidParams(
            "definition must not be empty".into(),
        ));
    }
    let saved = store
        .set_glossary_term(NewGlossaryTerm {
            workspace_id: auth.workspace_id,
            term: a.term.trim().to_string(),
            definition: a.definition,
            aliases: a.aliases.unwrap_or_default(),
            created_by: auth.member_id,
        })
        .await?;
    Ok(content_json(&saved))
}

#[derive(Deserialize)]
struct GetGlossaryArgs {
    term: String,
}

/// Look up one term's definition in the caller's workspace glossary (Cluster
/// 322). Returns `null` when the term is undefined.
pub(super) async fn get_glossary_term(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetGlossaryArgs = serde_json::from_value(args.clone())?;
    let term = store
        .get_glossary_term(auth.workspace_id, a.term.trim())
        .await?;
    Ok(content_json(&term))
}

/// All defined terms in the caller's workspace glossary, ordered by term
/// (Cluster 322).
pub(super) async fn list_glossary_terms(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let terms = store.list_glossary_terms(auth.workspace_id).await?;
    Ok(content_json(&terms))
}
