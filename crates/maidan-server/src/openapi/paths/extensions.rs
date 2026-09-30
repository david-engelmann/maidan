//! OpenAPI path stubs for routes whose spec entry lives here rather than in
//! `api.rs`.
//!
//! These are utoipa annotation carriers, not unwired routes: every one is
//! mounted in `app.rs` and covered by the `openapi_e2e` bijection against
//! `contracts/http-capability-map.json`. The banner used to read "missing from
//! `api.rs`", which scanned as "these routes are not implemented". The split is
//! purely where the annotation sits.

use crate::error::ProblemDetails;
use crate::openapi::responses::*;
use crate::reindex_ops::StartReindexEmbeddings;
use maidan_types::ReindexJob;
use uuid::Uuid;

use crate::dto::*;
use crate::status::OperatorStatus;
use crate::thread_context::WorkspaceContext;
use maidan_types::*;

/// Purge a workspace's retained content
#[utoipa::path(
    post,
    path = "/workspaces/{id}/purge",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Messages purged"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, description = "The workspace is under legal hold", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn purge_workspace() {}

/// Export a workspace
#[utoipa::path(
    get,
    path = "/workspaces/{id}/export",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Signed workspace export envelope (maidan.workspace.export/1 JSON)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn export_workspace() {}

/// Verify a signed workspace export
#[utoipa::path(
    post,
    path = "/workspaces/export/verify",
    tag = "workspaces",
    request_body(content = serde_json::Value, description = "A signed `maidan.workspace.export/1` envelope"),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = VerifyExportResult, description = "Signature valid; tokens die on export"),
        (status = 400, description = "Tamper, bad signature, secret fields, or key not in pin", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, response = Forbidden),
    )
)]
pub fn verify_workspace_export() {}

/// Get the operator export public key
#[utoipa::path(
    get,
    path = "/operator/export-public-key",
    tag = "operator",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ExportPublicKey, description = "Operator Ed25519 public key for export verify pins"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn export_public_key() {}

/// Import a signed workspace export
#[utoipa::path(
    post,
    path = "/workspaces/import",
    tag = "workspaces",
    params(ImportQuery),
    request_body(content = serde_json::Value, description = "A signed `maidan.workspace.export/1` envelope"),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ImportResult, description = "Imported after signature verify; body is a signed maidan.workspace.export/1 envelope"),
        (status = 400, description = "Signature invalid or bundle rejected", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, description = "restore mode: a workspace with the bundle's id already exists (retry with force=true)", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn import_workspace() {}

/// Get a workspace's usage counts
#[utoipa::path(
    get,
    path = "/workspaces/{id}/usage",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WorkspaceUsage),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_workspace_usage() {}

/// List a workspace's withdrawn messages
#[utoipa::path(
    get,
    path = "/workspaces/{id}/tombstones",
    tag = "workspaces",
    params(
        ("id" = Uuid, Path, description = "Workspace id"),
        ListTombstonesQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [TombstoneRecord], description = "Tombstoned messages, newest first; include_purged reconstructs hard deletes"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn list_workspace_tombstones() {}

/// Count a workspace's events by kind
#[utoipa::path(
    get,
    path = "/workspaces/{id}/kind-census",
    tag = "workspaces",
    params(
        ("id" = Uuid, Path, description = "Workspace id"),
        KindCensusQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = KindCensus, description = "EventKind counts for the workspace or a narrower scope"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_workspace_kind_census() {}

/// List a workspace's thread results
#[utoipa::path(
    get,
    path = "/workspaces/{id}/results",
    tag = "workspaces",
    params(
        ("id" = Uuid, Path, description = "Workspace id"),
        ListThreadResultsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [ThreadResult], description = "Workspace thread results, newest first; optional exact result_kind facet"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn list_workspace_results() {}

/// List a producer run's threads
#[utoipa::path(
    get,
    path = "/workspaces/{id}/run-threads",
    tag = "workspaces",
    params(
        ("id" = Uuid, Path, description = "Workspace id"),
        RunLineageQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [Thread], description = "Threads sharing a producer parent_run_id"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn list_run_threads() {}

/// Get a producer run's occupancy
#[utoipa::path(
    get,
    path = "/workspaces/{id}/run-occupancy",
    tag = "workspaces",
    params(
        ("id" = Uuid, Path, description = "Workspace id"),
        RunLineageQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = RunOccupancy, description = "Nested occupancy for a producer run"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_run_occupancy() {}

/// Place a legal hold on a workspace
#[utoipa::path(post, path = "/workspaces/{id}/legal-holds", tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = PlaceLegalHold,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = LegalHold, description = "A hold for one matter"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    ))]
pub fn place_legal_hold() {}

/// List a workspace's legal holds
#[utoipa::path(get, path = "/workspaces/{id}/legal-holds", tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [LegalHold], description = "The workspace's holds; empty when not held"),
        (status = 403, response = Forbidden),
    ))]
pub fn list_workspace_legal_holds() {}

/// Lift a legal hold
#[utoipa::path(delete, path = "/workspaces/{id}/legal-holds/{hold_id}", tag = "workspaces",
    params(
        ("id" = Uuid, Path, description = "Workspace id"),
        ("hold_id" = Uuid, Path, description = "Legal hold id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Lifted; the last lift disposes of what the holds kept"),
        (status = 403, response = Forbidden),
        (status = 404, description = "No such hold on this workspace", body = ProblemDetails, content_type = "application/problem+json"),
    ))]
pub fn lift_legal_hold() {}

/// Read what a workspace's legal holds preserved
#[utoipa::path(get, path = "/workspaces/{id}/legal-holds/preserved", tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [PreservedMessage],
        description = "Messages withdrawn while held: last words and earlier versions. Each read is audited"),
        (status = 403, response = Forbidden),
    ))]
pub fn get_preserved_messages() {}

/// List legal holds across all workspaces
#[utoipa::path(get, path = "/operator/legal-holds", tag = "operator",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [LegalHold], description = "Active legal holds across all workspaces"),
        (status = 403, response = Forbidden),
    ))]
pub fn list_legal_holds() {}

/// Get operator status
#[utoipa::path(get, path = "/operator/status", tag = "operator",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = OperatorStatus,
        description = "Phase, readiness checks, search backfill, replica lag and queue depths; `Accept: text/html` returns the same as a page"),
        (status = 403, response = Forbidden),
    ))]
pub fn operator_status() {}

/// Set a workspace's WIP limit
#[utoipa::path(
    put,
    path = "/workspaces/{id}/wip-limit",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = SetWipLimit,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WipLimitView, description = "The WIP limit (set or cleared)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn set_wip_limit() {}

/// Get a workspace's WIP limit
#[utoipa::path(
    get,
    path = "/workspaces/{id}/wip-limit",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WipLimitView, description = "The workspace WIP limit (null = unlimited)"),
        (status = 403, response = Forbidden),
    )
)]
pub fn get_wip_limit() {}

/// Set a workspace's delegation policy
#[utoipa::path(
    put,
    path = "/workspaces/{id}/delegation-policy",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = SetDelegationPolicy,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = DelegationPolicy, description = "The grant ceiling now in force"),
        (status = 400, description = "Ceiling outside 1–3650 days", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Requires token:admin", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn set_delegation_policy() {}

/// Get a workspace's delegation policy
#[utoipa::path(
    get,
    path = "/workspaces/{id}/delegation-policy",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = DelegationPolicy, description = "The longest a delegation grant may live"),
        (status = 403, response = Forbidden),
    )
)]
pub fn get_delegation_policy() {}

/// Set a workspace's retention
#[utoipa::path(
    put,
    path = "/workspaces/{id}/retention",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    request_body = RetentionDays,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = RetentionPolicy, description = "The workspace's retention, the instance's, and what is pruned in effect"),
        (status = 400, description = "Days outside 1–3650, or longer than the instance keeps that kind of row", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Requires token:admin", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn set_retention_policy() {}

/// Get a workspace's retention
#[utoipa::path(
    get,
    path = "/workspaces/{id}/retention",
    tag = "workspaces",
    params(("id" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = RetentionPolicy, description = "The workspace's retention, the instance's, and what is pruned in effect"),
        (status = 403, response = Forbidden),
    )
)]
pub fn get_retention_policy() {}

/// Get a member's work in progress
#[utoipa::path(
    get,
    path = "/members/{id}/wip",
    tag = "threads",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = MemberWipView, description = "The member's live-claim count vs the workspace limit"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_member_wip() {}

/// Get the caller's identity
#[utoipa::path(
    get,
    path = "/me",
    tag = "workspaces",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WhoAmI),
        (status = 403, response = Forbidden),
    )
)]
pub fn get_me() {}

/// List a workspace's glossary terms
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/glossary",
    tag = "workspaces",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<GlossaryTerm>, description = "All defined terms, ordered by term"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn list_glossary_terms() {}

/// Define a glossary term
#[utoipa::path(
    put,
    path = "/workspaces/{wid}/glossary/{term}",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("term" = String, Path, description = "The term being defined")
    ),
    request_body = SetGlossaryTerm,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = GlossaryTerm, description = "The defined term (upserted)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn set_glossary_term() {}

/// Get a glossary term
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/glossary/{term}",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("term" = String, Path, description = "The term to look up")
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = GlossaryTerm),
        (status = 403, response = Forbidden),
        (status = 404, description = "No such term is defined", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn get_glossary_term() {}

/// Delete a glossary term
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/glossary/{term}",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("term" = String, Path, description = "The term to remove")
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Removed"),
        (status = 403, response = Forbidden),
        (status = 404, description = "No such term is defined", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn delete_glossary_term() {}

/// List a member's assigned threads
#[utoipa::path(
    get,
    path = "/members/{id}/assigned-threads",
    tag = "threads",
    params(("id" = Uuid, Path, description = "Member id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Thread>, description = "Threads assigned to the member"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn list_assigned_threads() {}

/// Claim the next ready thread in a channel
#[utoipa::path(
    post,
    path = "/channels/{cid}/threads/claim-next",
    tag = "threads",
    params(("cid" = Uuid, Path, description = "Channel id")),
    request_body = ClaimNextThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ClaimedThread, description = "The claimed thread plus a content-addressed pin, or null when the channel has no claimable work"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn claim_next_thread() {}

/// Claim the next ready thread anywhere in a workspace
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/threads/claim-next",
    tag = "threads",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = ClaimNextThread,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = ClaimedThread, description = "The claimed thread plus a content-addressed pin, or null when no channel the caller may read has claimable work. The channel route's filters, order and lease apply; a private channel's threads go only to its members, a DM's only to its participants"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn claim_next_workspace_thread() {}

/// Renew a claim
#[utoipa::path(
    post,
    path = "/threads/{id}/claim/renew",
    tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = RenewClaim,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread, description = "Lease extended (current assignee only)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn renew_claim() {}

/// Acknowledge a claim
#[utoipa::path(
    post,
    path = "/threads/{id}/claim/acknowledge",
    tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = AcknowledgeClaim,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread, description = "Working clock started (current holder with the matching fencing token)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn acknowledge_claim() {}

/// Release a claim
#[utoipa::path(
    post,
    path = "/threads/{id}/claim/release",
    tag = "threads",
    params(("id" = Uuid, Path, description = "Thread id")),
    request_body = ReleaseClaim,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Thread, description = "Claim released back to the queue (current holder with the matching fencing token)"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn release_claim() {}

/// List a workspace's audit events
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/audit",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ListAuditQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<AuditEvent>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_workspace_audit() {}

/// Get a workspace's context pack
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/context",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        WorkspaceContextQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = WorkspaceContext),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_workspace_context() {}

/// List a workspace's quarantined outbox entries
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/outbox/quarantined",
    params(("wid" = Uuid, Path, description = "Workspace id"), crate::routes::QuarantinedOutboxQuery),
    tag = "workspaces",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Quarantined outbox rows"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_quarantined_outbox() {}

/// Replay a quarantined outbox entry
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/outbox/{oid}/replay",
    tag = "workspaces",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("oid" = i64, Path, description = "Outbox row id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Replay accepted"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn replay_quarantined_outbox() {}

/// Register an app
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/apps",
    tag = "apps",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = RegisterApp,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "App registered"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn register_app() {}

/// List a workspace's registered apps
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/apps",
    tag = "apps",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Installed apps"),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_apps() {}

/// Install an app
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/apps/{app_id}/install",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("app_id" = Uuid, Path, description = "App id")),
    request_body = InstallApp,
    tag = "apps",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "App installed"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn install_app() {}

/// Authorize an app installation
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/apps/{app_id}/oauth/authorize",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("app_id" = Uuid, Path, description = "App id")),
    request_body = crate::app_oauth::AuthorizeAppInstall,
    tag = "apps",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Authorization URL or redirect"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn authorize_app_install() {}

/// List a workspace's app installations
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/app-installations",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    tag = "apps",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Installations"),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_app_installations() {}

/// Revoke an app installation
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/app-installations/{iid}",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("iid" = Uuid, Path, description = "App installation id")),
    tag = "apps",
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Revoked"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn revoke_app_installation() {}

/// Mint a token for an app installation
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/app-installations/{iid}/tokens",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("iid" = Uuid, Path, description = "App installation id")),
    request_body = MintAppToken,
    tag = "apps",
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = MintApiTokenResponse),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn mint_app_token() {}

/// Open a direct-message conversation
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/dm",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    tag = "dm",
    security(("bearerAuth" = [])),
    request_body = crate::dm::OpenDmBody,
    responses(
        (status = 201, description = "DM conversation"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    )
)]
pub fn open_dm_conversation() {}

/// List a workspace's direct-message conversations
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/dm",
    params(("wid" = Uuid, Path, description = "Workspace id"), crate::dm::DmMemberQuery),
    tag = "dm",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "DM conversations"),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_dm_conversations() {}

/// Get a direct-message conversation
#[utoipa::path(
    get,
    path = "/dm/{id}",
    params(("id" = Uuid, Path, description = "DM conversation id")),
    tag = "dm",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "DM conversation"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_dm_conversation() {}

/// Post a direct message
#[utoipa::path(
    post,
    path = "/dm/{id}/messages",
    params(("id" = Uuid, Path, description = "DM conversation id")),
    tag = "dm",
    security(("bearerAuth" = [])),
    request_body = PostDmMessage,
    responses(
        (status = 201, body = Message),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    )
)]
pub fn post_dm_message() {}

/// List a direct-message conversation's messages
#[utoipa::path(
    get,
    path = "/dm/{id}/messages",
    params(("id" = Uuid, Path, description = "DM conversation id"), ListMessagesQuery),
    tag = "dm",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<Message>),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn list_dm_messages() {}

/// Open a group direct-message conversation
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/group-dms",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    tag = "dm",
    security(("bearerAuth" = [])),
    request_body = OpenGroupDmBody,
    responses(
        (status = 201, body = GroupDmConversation),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    )
)]
pub fn open_group_dm() {}

/// List a workspace's group direct-message conversations
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/group-dms",
    tag = "dm",
    security(("bearerAuth" = [])),
    params(("wid" = Uuid, Path, description = "Workspace id"), ("member_id" = uuid::Uuid, Query, description = "Member whose group conversations are listed")),
    responses(
        (status = 200, body = Vec<GroupDmConversation>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_group_dms() {}

/// Get a group direct-message conversation
#[utoipa::path(
    get,
    path = "/group-dms/{id}",
    params(("id" = Uuid, Path, description = "Group DM id")),
    tag = "dm",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = GroupDmConversation),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_group_dm() {}

/// Post to a group direct-message conversation
#[utoipa::path(
    post,
    path = "/group-dms/{id}/messages",
    params(("id" = Uuid, Path, description = "Group DM id")),
    tag = "dm",
    security(("bearerAuth" = [])),
    request_body = PostDmMessage,
    responses(
        (status = 201, body = Message),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, response = Conflict),
    )
)]
pub fn post_group_dm_message() {}

/// Purge a message
#[utoipa::path(
    delete,
    path = "/messages/{id}/purge",
    params(("id" = Uuid, Path, description = "Message id")),
    tag = "messages",
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Purged"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
        (status = 409, description = "The workspace is under legal hold", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn purge_message() {}

/// List dead-lettered automation deliveries
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/automation/dlq",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    tag = "automation",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Dead-letter deliveries"),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_automation_dlq() {}

/// List automation deliveries
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/automation/deliveries",
    params(("wid" = Uuid, Path, description = "Workspace id"), crate::automation_deliveries::ListAutomationDeliveriesQuery),
    tag = "automation",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Automation deliveries"),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_automation_deliveries() {}

/// Get an automation delivery
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/automation/deliveries/{did}",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("did" = i64, Path, description = "Delivery id")),
    tag = "automation",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Delivery row"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_automation_delivery() {}

/// Replay an automation delivery
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/automation/deliveries/{did}/replay",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("did" = i64, Path, description = "Delivery id")),
    tag = "automation",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Replay enqueued"),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn replay_automation_delivery() {}

/// List webhook and automation deliveries
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/deliveries",
    params(("wid" = Uuid, Path, description = "Workspace id"), crate::delivery_ops::ListDeliveriesQuery),
    tag = "operator",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Webhook + automation deliveries"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_unified_deliveries() {}

/// Get a webhook or automation delivery
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/deliveries/{did}",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("did" = i64, Path, description = "Delivery id"), crate::delivery_ops::DeliveryKindQuery),
    tag = "operator",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Delivery row"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn get_unified_delivery() {}

/// Replay a webhook or automation delivery
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/deliveries/{did}/replay",
    params(("wid" = Uuid, Path, description = "Workspace id"), ("did" = i64, Path, description = "Delivery id"), crate::delivery_ops::DeliveryKindQuery),
    tag = "operator",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Replay enqueued"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn replay_unified_delivery() {}

/// List audit events across all workspaces
#[utoipa::path(
    get,
    path = "/operator/audit",
    tag = "operator",
    params(("limit" = Option<i64>, Query, description = "Max events (default 50, clamped 1..=500)")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Audit events across all workspaces"),
        (status = 403, description = "Missing audit:read-global capability", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn list_global_audit() {}

/// Start an embeddings reindex
#[utoipa::path(
    post,
    path = "/operator/reindex-embeddings",
    tag = "operator",
    request_body = StartReindexEmbeddings,
    security(("bearerAuth" = [])),
    responses(
        (status = 202, description = "Reindex job accepted", body = ReindexJob),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn start_reindex_embeddings() {}

/// Get an embeddings reindex job
#[utoipa::path(
    get,
    path = "/operator/reindex-embeddings/{job_id}",
    tag = "operator",
    params(("job_id" = Uuid, Path, description = "Reindex job id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Job status", body = ReindexJob),
        (status = 403, response = Forbidden),
        (status = 404, description = "Unknown job", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn get_reindex_embeddings_job() {}

/// List dead-lettered mail
#[utoipa::path(
    get,
    path = "/operator/mail/dead",
    tag = "operator",
    params(("limit" = Option<i64>, Query, description = "Max entries (default 100, clamped 1..=500)")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Dead-lettered notification emails, newest first"),
        (status = 403, description = "Missing token:admin capability", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn list_dead_mail() {}

/// Requeue dead-lettered mail
#[utoipa::path(
    post,
    path = "/operator/mail/dead/{id}/requeue",
    tag = "operator",
    params(("id" = Uuid, Path, description = "Dead mail-outbox entry id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Requeued for another delivery attempt"),
        (status = 403, response = Forbidden),
        (status = 404, description = "No dead entry with that id", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn requeue_dead_mail() {}

/// List dead-lettered projector deliveries
#[utoipa::path(
    get,
    path = "/operator/egress/dead",
    tag = "operator",
    params(("limit" = Option<i64>, Query, description = "Max entries (default 100, clamped 1..=500)")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Dead-lettered projector deliveries, newest first", body = Vec<DeadEgress>),
        (status = 403, description = "Missing token:admin capability", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn list_dead_egress() {}

/// Requeue a dead-lettered projector delivery
#[utoipa::path(
    post,
    path = "/operator/egress/dead/{id}/requeue",
    tag = "operator",
    params(("id" = Uuid, Path, description = "Dead egress-outbox entry id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Requeued for another delivery attempt"),
        (status = 403, response = Forbidden),
        (status = 404, description = "No dead entry with that id", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn requeue_dead_egress() {}

/// Allow an egress target
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/egress-targets",
    tag = "integrations",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = AllowEgressTarget,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, description = "Destination blessed (idempotent: a re-bless returns the existing entry)", body = AllowedEgressTarget),
        (status = 400, description = "Selector is a name rather than an id (a Slack #name, or owner/name#123)", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Missing token:admin capability", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn allow_egress_target() {}

/// List a workspace's egress targets
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/egress-targets",
    tag = "integrations",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Blessed destinations; empty (the default) means deliver nowhere", body = Vec<AllowedEgressTarget>),
        (status = 403, description = "Missing token:admin capability", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn list_egress_targets() {}

/// Revoke an egress target
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/egress-targets/{tid}",
    tag = "integrations",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("tid" = Uuid, Path, description = "Allowlist entry id"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Blessing revoked"),
        (status = 403, response = Forbidden),
        (status = 404, description = "This workspace has no such entry", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn revoke_egress_target() {}

/// Link a Slack channel to a thread
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/slack-links",
    tag = "integrations",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = LinkSlackChannel,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = SlackChannelLink),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn link_slack_channel() {}

/// List a workspace's Slack links
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/slack-links",
    tag = "integrations",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<SlackChannelLink>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_slack_channel_links() {}

/// Unlink a Slack channel
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/slack-links/{slack_channel_id}",
    tag = "integrations",
    params(
        ("wid" = Uuid, Path, description = "Workspace id"),
        ("slack_channel_id" = String, Path, description = "Slack channel id")
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Unlinked"),
        (status = 403, response = Forbidden),
        (status = 404, description = "No such link", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn unlink_slack_channel() {}

/// Link a GitHub issue to a thread
#[utoipa::path(
    post,
    path = "/workspaces/{wid}/github-links",
    tag = "integrations",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    request_body = LinkGithubIssue,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = GithubIssueLink),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, response = NotFound),
    )
)]
pub fn link_github_issue() {}

/// List a workspace's GitHub links
#[utoipa::path(
    get,
    path = "/workspaces/{wid}/github-links",
    tag = "integrations",
    params(("wid" = Uuid, Path, description = "Workspace id")),
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = Vec<GithubIssueLink>),
        (status = 403, response = Forbidden),
    )
)]
pub fn list_github_issue_links() {}

/// Unlink a GitHub issue
#[utoipa::path(
    delete,
    path = "/workspaces/{wid}/github-links",
    tag = "integrations",
    params(("wid" = Uuid, Path, description = "Workspace id"), UnlinkGithubQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = 204, description = "Unlinked"),
        (status = 400, response = BadRequest),
        (status = 403, response = Forbidden),
        (status = 404, description = "No such link", body = ProblemDetails, content_type = "application/problem+json"),
    )
)]
pub fn unlink_github_issue() {}
