//! Backend-neutral pieces of `approval_decide`'s store: the threshold a
//! workspace with no row gets, and how a decision's record is read back.

use maidan_types::{
    ApprovalConfirmation, ApprovalPolicy, ApprovalRisk, ClientIdentitySource, GateDecisionVia,
    WorkspaceId,
};

/// The threshold when a workspace has set none. `low` makes every accept
/// through the tool need a person's confirmation, so a workspace has to opt
/// in to any direct accept.
pub const DEFAULT_CONFIRM_AT: ApprovalRisk = ApprovalRisk::Low;

/// The policy for a stored `confirm_at`, or the default for none. A value the
/// CHECK constraint would refuse cannot be read back, and if one were it
/// reads as the default, which is the strictest.
pub(crate) fn policy(workspace_id: WorkspaceId, stored: Option<&str>) -> ApprovalPolicy {
    match stored.and_then(ApprovalRisk::parse) {
        Some(confirm_at) => ApprovalPolicy {
            workspace_id,
            confirm_at,
            is_default: false,
        },
        None => ApprovalPolicy {
            workspace_id,
            confirm_at: DEFAULT_CONFIRM_AT,
            is_default: true,
        },
    }
}

/// A stored client source. Anything the CHECK would refuse, or none at all,
/// reads as `none`: the weakest claim, never a stronger one.
pub(crate) fn source(stored: Option<String>) -> ClientIdentitySource {
    stored
        .as_deref()
        .and_then(ClientIdentitySource::parse)
        .unwrap_or_default()
}

/// A gate's decision record, present only when a model asked.
pub(crate) fn decided_via(
    client_name: Option<String>,
    client_version: Option<String>,
    client_id: Option<String>,
    client_source: Option<String>,
    model_asked: bool,
) -> Option<GateDecisionVia> {
    model_asked.then(|| GateDecisionVia {
        client_name,
        client_version,
        client_id,
        client_source: source(client_source),
        model_asked,
    })
}

/// The decision record a confirmed request leaves: the client the model
/// asked through, never the console the person confirmed in.
pub(crate) fn via(confirmation: &ApprovalConfirmation) -> GateDecisionVia {
    GateDecisionVia {
        client_name: confirmation.client_name.clone(),
        client_version: confirmation.client_version.clone(),
        client_id: confirmation.client_id.clone(),
        client_source: confirmation.client_source,
        model_asked: true,
    }
}

/// The gate's `content` for a decision note, in the shape the tool writes.
pub fn note_content(note: Option<&str>) -> Option<serde_json::Value> {
    note.map(|note| serde_json::json!({ "note": note }))
}
