//! Shared-glossary MCP tools: an agent defines, looks up, lists, and deletes a
//! workspace canonical `term -> definition` (+ aliases). Workspace-scoped:
//! `set` and `delete` are `workspace:write`; `get`/`list` are `workspace:read`.
//! `delete_glossary_term` calls the same store function as REST DELETE.
//! No channel/thread arg, so no pre-dispatch access gate — the workspace cap
//! is the control.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetGlossaryArgs {
    term: String,
    definition: String,
    #[serde(default)]
    aliases: Option<Vec<String>>,
}

/// Define (or redefine) a term in the caller's workspace glossary. Upserts on
/// `(workspace, term)`; owned by the caller.
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
#[serde(deny_unknown_fields)]
struct GetGlossaryArgs {
    term: String,
}

/// Look up one term's definition in the caller's workspace glossary. Returns
/// `null` when the term is undefined.
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

/// All defined terms in the caller's workspace glossary, ordered by term.
pub(super) async fn list_glossary_terms(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let terms = store.list_glossary_terms(auth.workspace_id).await?;
    Ok(content_json(&terms))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteGlossaryArgs {
    term: String,
}

/// Remove a glossary term. Twin of `DELETE /workspaces/{wid}/glossary/{term}`.
/// `NotFound` when the term is not defined.
pub(super) async fn delete_glossary_term(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: DeleteGlossaryArgs = serde_json::from_value(args.clone())?;
    let deleted = store
        .delete_glossary_term(auth.workspace_id, a.term.trim())
        .await?;
    if !deleted {
        return Err(McpError::NotFound);
    }
    Ok(content_json(&serde_json::json!({ "deleted": true })))
}
