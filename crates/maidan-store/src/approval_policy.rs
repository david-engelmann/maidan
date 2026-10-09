//! Backend-neutral pieces of `approval_decide`'s store: the threshold and
//! link lifetime a workspace with no row gets, and how a decision's record is
//! read back.

use maidan_types::{
    ApprovalConfirmation, ApprovalPolicy, ApprovalRisk, GateDecisionVia, WorkspaceId,
};

/// The threshold when a workspace has set none. `low` makes every accept
/// through the tool need a person's confirmation, so a workspace has to opt
/// in to any direct accept.
pub const DEFAULT_CONFIRM_AT: ApprovalRisk = ApprovalRisk::Low;

/// How long a confirmation link lives when the workspace has set nothing:
/// ten minutes.
pub const DEFAULT_CONFIRM_LINK_TTL_SECONDS: u32 = 600;

/// The shortest and longest lifetime a workspace may set, which the
/// column's CHECK constraint (migration 0149) also holds.
pub const CONFIRM_LINK_TTL_SECONDS_RANGE: std::ops::RangeInclusive<u32> = 60..=3600;

/// The policy for a stored row, or the defaults for none. A value the CHECK
/// constraints would refuse cannot be read back. If a threshold were, it
/// reads as the default, which is the strictest; a lifetime outside the
/// range reads as the default too.
pub(crate) fn policy(workspace_id: WorkspaceId, stored: Option<(String, i64)>) -> ApprovalPolicy {
    let Some((confirm_at, ttl)) = stored else {
        return ApprovalPolicy {
            workspace_id,
            confirm_at: DEFAULT_CONFIRM_AT,
            confirm_link_ttl_seconds: DEFAULT_CONFIRM_LINK_TTL_SECONDS,
            is_default: true,
        };
    };
    ApprovalPolicy {
        workspace_id,
        confirm_at: ApprovalRisk::parse(&confirm_at).unwrap_or(DEFAULT_CONFIRM_AT),
        confirm_link_ttl_seconds: u32::try_from(ttl)
            .ok()
            .filter(|t| CONFIRM_LINK_TTL_SECONDS_RANGE.contains(t))
            .unwrap_or(DEFAULT_CONFIRM_LINK_TTL_SECONDS),
        is_default: false,
    }
}

/// A gate's decision record, present only when a model asked.
pub(crate) fn decided_via(
    client_name: Option<String>,
    client_version: Option<String>,
    model_asked: bool,
) -> Option<GateDecisionVia> {
    model_asked.then_some(GateDecisionVia {
        client_name,
        client_version,
        model_asked,
    })
}

/// The decision record a confirmed request leaves: the client the model
/// asked through, never the console the person confirmed in.
pub(crate) fn via(confirmation: &ApprovalConfirmation) -> GateDecisionVia {
    GateDecisionVia {
        client_name: confirmation.client_name.clone(),
        client_version: confirmation.client_version.clone(),
        model_asked: true,
    }
}

/// The gate's `content` for a decision note, in the shape the tool writes.
pub fn note_content(note: Option<&str>) -> Option<serde_json::Value> {
    note.map(|note| serde_json::json!({ "note": note }))
}
