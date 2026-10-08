//! Human-in-the-loop approval tools.
//!
//! `request_approval` opens a durable
//! [`ApprovalGate`](maidan_types::ApprovalGate) and returns an `input_required`
//! result **without blocking** — the agent is free to do other work and poll
//! `get_approval_gate` (or await the `ThreadResultSet`-style signal in a later
//! cluster) for the human's decision. This replaces the pre-350 model where the
//! tool call parked up to 30s on an in-memory oneshot: the gate is now
//! persisted, survives a dropped connection, and is answerable by a human over
//! the `/ui`. "Silence is not consent" — a gate is resolved only by an explicit
//! accept/decline/cancel.

use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::call_context::CallContext;
use crate::error::McpError;
use maidan_auth::{capability::APPROVAL_GRANT, AuthContext};
use maidan_types::{
    ApprovalGate, ApprovalGateId, ApprovalGateState, ApprovalRisk, AuditScope, GateDecisionVia,
    MemberKind, NewApprovalConfirmation, NewApprovalGate, NewAuditEvent,
};

/// Unknown fields are rejected.
///
/// `thread_id` is load-bearing by its absence: supplied, the pending gate
/// blocks `claim_next` so no agent picks the thread up until a human answers;
/// omitted, the gate is unattached and gates nothing. A misspelled key was
/// indistinguishable from deliberate omission, so `threadId` or `thread`
/// returned `200` with a gate that looks right and blocks nobody — the agent
/// claims the thread and proceeds without the human. Same shape as the budget
/// dimensions: a typo silently disarms a control.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestApprovalArgs {
    /// Human-readable description of what needs approval.
    prompt: String,
    /// Optional JSON Schema for structured detail the human may supply alongside
    /// their decision (MCP `requestedSchema`). Persisted on the gate.
    #[serde(default)]
    schema: Option<Value>,
    /// Optional thread to attach the gate to. While the gate is `pending`,
    /// `claim_next` will not hand the thread to an agent — the required human
    /// is a *claim gate*, not a notification preference.
    #[serde(default)]
    thread_id: Option<uuid::Uuid>,
    /// How much a wrong accept would cost. Omitted means `high`. The
    /// workspace's approval policy decides from this whether a model's accept
    /// through `approval_decide` needs a person to confirm it.
    #[serde(default)]
    risk: ApprovalRisk,
}

/// Open a durable human-approval gate and return `input_required`.
///
/// The gate is created `pending` and attributed to the caller
/// (`auth.member_id`); a human resolves it later to accept/decline/cancel. The
/// tool does **not** block — poll `get_approval_gate` with the returned
/// `gate_id` for the outcome. Requires `workspace:read`: asking a human is part
/// of an agent's read-only reasoning loop, and only the human's answer
/// (`POST /approval-gates/:id/answer`) needs `workspace:write`.
pub(super) async fn request_approval(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RequestApprovalArgs = crate::tools::parse_args(args)?;
    let (gate, event) = server
        .store
        .create_approval_gate_with_event(&NewApprovalGate {
            workspace_id: auth.workspace_id,
            thread_id: a.thread_id.map(maidan_types::ThreadId),
            requested_by: auth.member_id,
            prompt: a.prompt,
            schema: a.schema,
            risk: a.risk,
        })
        .await?;
    server.publish_stored(&event).await;
    Ok(content_json(&json!({
        "status": "input_required",
        "gate_id": gate.id,
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetApprovalGateArgs {
    gate_id: ApprovalGateId,
}

/// Poll a durable approval gate by id. Returns the gate — its `state` is
/// `pending` until a human answers, then `accepted`/`declined`/ `cancelled`
/// with any `content` they supplied. `null` if no such gate exists in the
/// caller's workspace. Requires `workspace:read`.
pub(super) async fn get_approval_gate(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetApprovalGateArgs = crate::tools::parse_args(args)?;
    let gate = server.store.get_approval_gate(a.gate_id).await?;
    match gate {
        // Scope to the caller's workspace (bypass sees all); an out-of-workspace
        // id reads as "not found" — no cross-tenant existence oracle.
        Some(g) if auth.bypass || g.workspace_id == auth.workspace_id => {
            Ok(content_json(&serde_json::to_value(g)?))
        }
        _ => Ok(content_json(&Value::Null)),
    }
}

/// The longest decision note kept on a gate.
const MAX_NOTE_CHARS: usize = 2000;

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Accept,
    Decline,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalDecideArgs {
    gate_id: ApprovalGateId,
    decision: Decision,
    #[serde(default)]
    note: Option<String>,
}

/// Decide a pending approval gate on a model's behalf.
///
/// Declining needs what declining over REST needs. Accepting is a property of
/// the credential (Next 17, decided 2026-10-08): a token holding
/// `approval:grant` accepts directly, and only on a gate whose risk is below
/// the workspace's confirmation threshold. Every other accept, a person's
/// plain bearer token above all, gets a one-time link the person opens in the
/// console and confirms through their own signed-in session. The model cannot
/// complete that step: it holds no session, and the page refuses a request
/// that did not come from the console. Nobody accepts their own request.
///
/// The decision, or the request for one, records the client the model asked
/// through and that a model asked.
pub(super) async fn approval_decide(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
    call: &CallContext,
) -> Result<Value, McpError> {
    let a: ApprovalDecideArgs = crate::tools::parse_args(args)?;
    let note = match a.note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
        Some(n) if n.chars().count() > MAX_NOTE_CHARS => {
            return Err(McpError::InvalidParams(format!(
                "note: at most {MAX_NOTE_CHARS} characters"
            )))
        }
        other => other,
    };
    // An out-of-workspace or unknown id reads as not found, so no tenant can
    // learn another's gate ids.
    let gate = server
        .store
        .get_approval_gate(a.gate_id)
        .await?
        .filter(|g| auth.bypass || g.workspace_id == auth.workspace_id)
        .ok_or_else(|| {
            McpError::InvalidParams("gate_id: no such approval gate in your workspace".into())
        })?;
    if gate.state.is_resolved() {
        return Ok(already_resolved(&gate));
    }
    let via = GateDecisionVia {
        client_name: call.client.name.clone(),
        client_version: call.client.version.clone(),
        model_asked: true,
    };
    if a.decision == Decision::Decline {
        return resolve(server, auth, &gate, ApprovalGateState::Declined, note, &via, "direct")
            .await;
    }
    if !auth.bypass {
        let asked = [Some(gate.requested_by), gate.requested_actor_id];
        let answering = [auth.member_id, auth.actor_id];
        if answering.iter().any(|m| asked.contains(&Some(*m))) {
            return Err(McpError::Forbidden(
                "an approval cannot be granted by whoever requested it".into(),
            ));
        }
    }
    let policy = server.store.get_approval_policy(gate.workspace_id).await?;
    let needs_confirmation = policy.needs_confirmation(gate.risk);
    // Auth disabled has no person to confirm and no credential to judge, as
    // on REST.
    if auth.bypass || (auth.has_capability(APPROVAL_GRANT) && !needs_confirmation) {
        return resolve(
            server,
            auth,
            &gate,
            ApprovalGateState::Accepted,
            note,
            &via,
            "approval:grant",
        )
        .await;
    }
    confirmation(server, auth, &gate, note, call, needs_confirmation).await
}

/// A gate that is no longer pending, said as a result rather than an error:
/// a model that retries after the person confirmed reads the outcome here.
fn already_resolved(gate: &ApprovalGate) -> Value {
    content_json(&json!({
        "status": "already_resolved",
        "state": gate.state,
        "gate": gate,
    }))
}

async fn resolve(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    gate: &ApprovalGate,
    target: ApprovalGateState,
    note: Option<String>,
    via: &GateDecisionVia,
    path: &'static str,
) -> Result<Value, McpError> {
    let content = maidan_store::approval_policy::note_content(note.as_deref());
    let actor = auth.actor_id;
    let audit_via = via.clone();
    let resolved = server
        .store
        .resolve_approval_gate_audited(
            gate.id,
            auth.member_id,
            target,
            content.as_ref(),
            via,
            Box::new(move |g: &ApprovalGate| NewAuditEvent {
                scope: AuditScope::Workspace(g.workspace_id),
                actor_id: Some(actor),
                action: "approval_gate.decided".into(),
                target_kind: Some("approval_gate".into()),
                target_id: Some(g.id.0),
                metadata: json!({
                    "surface": "mcp",
                    "tool": "approval_decide",
                    "decision": g.state,
                    "path": path,
                    "risk": g.risk,
                    "client_name": audit_via.client_name,
                    "client_version": audit_via.client_version,
                    "model_asked": audit_via.model_asked,
                }),
            }),
        )
        .await?;
    match resolved {
        Some(g) => Ok(content_json(&json!({ "status": g.state, "gate": g }))),
        // Someone answered between the read and the compare-and-set.
        None => match server.store.get_approval_gate(gate.id).await? {
            Some(g) => Ok(already_resolved(&g)),
            None => Err(McpError::NotFound),
        },
    }
}

/// The confirmation path: a live one-time link for this gate and member, as
/// a URL-mode elicitation where the client can show one, or in the result.
async fn confirmation(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    gate: &ApprovalGate,
    note: Option<String>,
    call: &CallContext,
    needs_confirmation: bool,
) -> Result<Value, McpError> {
    // A link is for the person whose credential this is. An agent member
    // cannot sign in to the console, so a link for one would be a dead end.
    let member = server.store.get_member(auth.member_id).await?;
    if member.kind != MemberKind::Human {
        return Err(McpError::Forbidden(
            "an agent's token accepts an approval gate only with approval:grant, and only \
             below the workspace's confirmation threshold: no person can confirm for an agent"
                .into(),
        ));
    }
    let keys = server.approval_confirmations().ok_or_else(|| {
        McpError::Internal(
            "approval confirmations are not configured on this server: accept the gate in \
             the Maidan console"
                .into(),
        )
    })?;
    let now = chrono::Utc::now();
    let nonce = uuid::Uuid::now_v7();
    let token = maidan_auth::approval_confirmation::token(&keys.secret, nonce);
    let ttl = chrono::Duration::from_std(keys.ttl)
        .map_err(|_| McpError::Internal("confirmation lifetime out of range".into()))?;
    let actor = auth.actor_id;
    let new = NewApprovalConfirmation {
        gate_id: gate.id,
        member_id: auth.member_id,
        workspace_id: gate.workspace_id,
        actor_id: (auth.actor_id != auth.member_id).then_some(auth.actor_id),
        nonce,
        token_hash: maidan_auth::approval_confirmation::token_hash(&token),
        client_name: call.client.name.clone(),
        client_version: call.client.version.clone(),
        note,
        now,
        expires_at: now + ttl,
    };
    let risk = gate.risk;
    let (confirmation, fresh) = server
        .store
        .issue_approval_confirmation(
            &new,
            Box::new(move |c| NewAuditEvent {
                scope: AuditScope::Workspace(c.workspace_id),
                actor_id: Some(actor),
                action: "approval_gate.confirmation_requested".into(),
                target_kind: Some("approval_gate".into()),
                target_id: Some(c.gate_id.0),
                metadata: json!({
                    "surface": "mcp",
                    "tool": "approval_decide",
                    "member_id": c.member_id,
                    "risk": risk,
                    "client_name": c.client_name,
                    "client_version": c.client_version,
                    "model_asked": true,
                    "expires_at": c.expires_at,
                }),
            }),
        )
        .await?;
    // A repeat call while the link is live gets that link back: the token is
    // derived from the stored nonce, so nothing new is minted.
    let token = if fresh {
        token
    } else {
        maidan_auth::approval_confirmation::token(&keys.secret, confirmation.nonce)
    };
    let url = keys.link(gate.id, &token);
    let why = if needs_confirmation {
        "this gate's risk is at or above the workspace's confirmation threshold"
    } else {
        "this credential cannot accept a gate on its own"
    };
    let message = format!(
        "A model asked to accept the approval gate \"{}\". Open the Maidan console to review \
         it and confirm, signed in as yourself.",
        truncate(&gate.prompt, 200)
    );
    // URL mode only, never a form: a form's answer comes back through the
    // client, which the model or client could fill in. The URL needs an
    // origin to be a valid one, and a retry answering this request gets the
    // plain result, so a client never loops on the same elicitation.
    if call.url_elicitation && !call.answers_input && keys.console_origin.is_some() {
        return Ok(json!({
            "resultType": "input_required",
            "inputRequests": {
                "confirm_in_console": {
                    "method": "elicitation/create",
                    "params": {
                        "mode": "url",
                        "url": url,
                        "message": message,
                        "elicitationId": format!("{}:{}", gate.id.0, confirmation.nonce),
                    }
                }
            }
        }));
    }
    Ok(content_json(&json!({
        "status": "confirmation_required",
        "gate_id": gate.id,
        "confirmation_url": url,
        "expires_at": confirmation.expires_at,
        "reused": !fresh,
        "why": why,
        "next": "Give the person this link. They confirm in the console, signed in as \
                 themselves; you cannot confirm for them. Do not call approval_decide again \
                 for this gate: poll get_approval_gate for the outcome.",
    })))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod arg_strictness_tests {
    use super::RequestApprovalArgs;

    /// A misspelled `thread_id` must not read as an omission.
    ///
    /// Omission is meaningful here — an unattached gate blocks nothing, so
    /// before this a `threadId` typo produced a `200` and a gate that looks
    /// correct while `claim_next` hands the thread straight to an agent.
    #[test]
    fn a_misspelled_thread_id_is_rejected_rather_than_leaving_the_gate_unattached() {
        for key in ["threadId", "thread", "thread_uuid"] {
            let args = serde_json::json!({ "prompt": "ship it?", key: uuid::Uuid::nil() });
            let err = serde_json::from_value::<RequestApprovalArgs>(args)
                .expect_err("a misspelled thread_id must not parse as an omission");
            assert!(
                err.to_string().contains(key),
                "the error should name the offending key {key}, got: {err}"
            );
        }
    }

    /// Deliberately omitting `thread_id` still means "a standalone gate".
    #[test]
    fn omitting_thread_id_still_means_an_unattached_gate() {
        let args = serde_json::json!({ "prompt": "ship it?" });
        let parsed: RequestApprovalArgs = serde_json::from_value(args).expect("parses");
        assert!(parsed.thread_id.is_none());
        assert!(parsed.schema.is_none());
    }
}
