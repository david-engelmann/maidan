//! MCP tools backed by [`maidan_store::Store`]. Each tool has a JSON
//! schema (input shape) and a dispatcher that decodes args, calls the
//! store, and returns a JSON result.
//!
//! The per-tool handlers are organized by domain in the submodules
//! below; the three entry points (`required_capability`, `catalog`,
//! `dispatch`) and the shared [`content_json`] helper live here.

use maidan_auth::capability::{
    ARTIFACT_UPLOAD, MEMBER_SELF, MESSAGE_POST, SEARCH_QUERY, SECRET_ADMIN, SECRET_READ,
    TOKEN_ADMIN, WORKSPACE_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE,
};
use maidan_auth::AuthContext;
use serde_json::{json, Value};

use crate::error::McpError;

mod approval;
mod artifact;
mod automation;
mod budget;
mod catalog;
mod change_pull;
mod channel;
mod credits;
mod delivery;
mod event_log;
mod event_wait;
mod explorer;
mod export;
mod freeze;
mod glossary;
mod id_scope;
mod land_gate;
mod member;
mod memory_block;
mod message;
mod projector;
mod recipe;
mod reference;
mod review;
mod room;
mod schedule;
mod search;
mod secret;
mod seed;
mod share;
mod skill;
mod snapshot;
mod social;
mod spawn;
mod thread;
mod thread_lookup;
mod whoami;

#[cfg(test)]
mod agent_gap_tests;

pub use catalog::{catalog, declared_arguments};

/// The tool catalog filtered to the tools the caller may invoke. A tool whose
/// required capability the caller lacks is omitted, so a capability-scoped
/// agent gets a smaller, relevant `tools/list` — fewer tokens and no tools it
/// would only get 403s from. The unfiltered [`catalog`] is unchanged (contract tests +
/// full-capability callers rely on it).
pub fn catalog_for(auth: &AuthContext) -> Vec<Value> {
    catalog()
        .into_iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(|n| n.as_str())
                .and_then(|name| required_capability(name).ok())
                .is_some_and(|cap| auth.has_capability(cap))
        })
        .collect()
}

/// Tools a workspace cut off on `mcp_calls` may still call: reading its
/// credits and opening a top-up intent, which moves no money, so an agent
/// past its allowance can pay its own way back.
pub const REACHABLE_PAST_CUTOFF: &[&str] = &["get_credits", "top_up_credits"];

/// Tools that change nothing. Every other tool is treated as a change and, if
/// it leaves no record of its own, gets one written for it by
/// [`crate::server::McpServer`]. Listing a tool here is a claim that it writes
/// nothing: getting it wrong in this direction loses a record, so the list is
/// explicit rather than a prefix rule a future `get_or_create_*` would satisfy.
/// Exporting a workspace and resolving a secret are reads that are
/// deliberately absent — who took the data is part of the record.
pub const READ_ONLY_TOOLS: &[&str] = &[
    "advise_land_gate",
    "catch_up_events",
    "find_thread",
    "find_threads",
    "get_approval_gate",
    "get_artifact",
    "get_artifact_metadata",
    "get_channel_occupancy",
    "get_credits",
    "get_delegation_policy",
    "get_delivery_mode",
    "get_dependency_results",
    "get_glossary_term",
    "get_group_dm",
    "get_inbox",
    "get_kind_census",
    "get_land_gate",
    "get_log_snapshot",
    "get_manager_digest",
    "get_member",
    "get_member_email",
    "get_member_occupancy",
    "get_member_wip",
    "get_memory_block",
    "get_priority",
    "get_queue_depth",
    "get_retention_policy",
    "get_review_packet",
    "get_review_requirement",
    "get_review_status",
    "get_room",
    "get_run_occupancy",
    "get_spawn_budget",
    "get_swarm_metrics",
    "get_thread_block",
    "get_thread_budget",
    "get_thread_context",
    "get_thread_lineage",
    "get_thread_result",
    "get_thread_status",
    "get_thread_steer",
    "get_thread_version",
    "get_tool_transcript",
    "get_unread_count",
    "get_wait",
    "get_waiting_inbox",
    "get_wip_limit",
    "get_workspace_context",
    "get_workspace_occupancy",
    "get_workspace_queue_depth",
    "list_active_claims",
    "list_assigned_threads",
    "list_blocked_threads",
    "list_buried_decisions",
    "list_capability_sets",
    "list_channel_follows",
    "list_channel_members",
    "list_channels",
    "list_child_threads",
    "list_delegation_grants",
    "list_dlq",
    "list_dm_conversations",
    "list_evidence_records",
    "list_frozen_members",
    "list_fsm_hooks",
    "list_github_issue_links",
    "list_glossary_terms",
    "list_group_dms",
    "list_land_gate_history",
    "list_member_follows",
    "list_member_skills",
    "list_members",
    "list_memory_blocks",
    "list_mentions",
    "list_message_backlinks",
    "list_message_edits",
    "list_messages",
    "list_notification_prefs",
    "list_notifications",
    "list_notifications_grouped",
    "list_pins",
    "list_reactions",
    "list_recently_active_threads",
    "list_recipes",
    "list_references",
    "list_result_deliveries",
    "list_review_history",
    "list_reviewers",
    "list_reviews",
    "list_run_threads",
    "list_secret_egress_hosts",
    "list_secrets",
    "list_share_tickets",
    "list_slack_channel_links",
    "list_slash_commands",
    "list_task_schedules",
    "list_thread_artifacts",
    "list_thread_dependencies",
    "list_thread_dependents",
    "list_thread_follows",
    "list_thread_memory_blocks",
    "list_thread_required_skills",
    "list_thread_results",
    "list_threads",
    "list_tombstones",
    "list_unclaimable",
    "list_votes",
    "parse_maidan_uri",
    "search_messages",
    "usage_rollup",
    "verify_event_chain",
    "verify_workspace_export",
    "wait_for_blocked_resolved",
    "wait_for_claim_expired",
    "wait_for_claim_failed",
    "wait_for_event",
    "wait_for_landed",
    "wait_for_memory_block",
    "wait_for_mention",
    "wait_for_notification",
    "wait_for_ready",
    "wait_for_result",
    "whoami",
];

/// How long a tool call may run before it is abandoned. Without one, a call
/// stuck on anything other than the database — which `statement_timeout`
/// bounds — held its connection and its client forever. Long-poll tools block
/// by design and bound themselves at five minutes, so they get that plus slack;
/// so do the bulk tools, whose work grows with the workspace. A call past its
/// deadline is dropped, which rolls back any transaction it had open rather
/// than leaving it half-applied.
pub fn deadline(name: &str) -> std::time::Duration {
    if name.starts_with("wait_for_") || BULK_TOOLS.contains(&name) {
        LONG_POLL_TOOL_DEADLINE
    } else {
        TOOL_DEADLINE
    }
}

const TOOL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// Tools whose work scales with the workspace rather than the request.
const BULK_TOOLS: &[&str] = &[
    "export_workspace",
    "get_log_snapshot",
    "import_workspace",
    "verify_event_chain",
];
const LONG_POLL_TOOL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(330);

/// A tool's arguments as `T`, or an invalid-params error that names the
/// argument at fault (`workspace_id: UUID parsing failed ...`), so a caller can
/// fix the call without reading the schema.
pub(crate) fn parse_args<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, McpError> {
    serde_path_to_error::deserialize(args).map_err(|err| {
        let path = err.path().to_string();
        let inner = err.into_inner();
        if path == "." {
            McpError::InvalidParams(inner.to_string())
        } else {
            McpError::InvalidParams(format!("{path}: {inner}"))
        }
    })
}

/// A tool that declares no arguments refuses any, as every argument struct
/// refuses an unknown field. Without this, an argument the caller believed
/// scoped the call, such as a `workspace_id` passed to `list_recipes`, was
/// dropped and the call answered for the caller's own workspace instead.
fn refuse_arguments_to_a_tool_that_takes_none(name: &str, args: &Value) -> Result<(), McpError> {
    let takes_none = declared_arguments(name).is_some_and(|declared| declared.is_empty());
    match args.as_object().and_then(|given| given.keys().next()) {
        Some(key) if takes_none => Err(McpError::InvalidParams(format!(
            "unknown field `{key}`: {name} takes no arguments"
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod parse_args_tests {
    use serde_json::json;

    use super::{parse_args, refuse_arguments_to_a_tool_that_takes_none};

    #[derive(Debug, serde::Deserialize)]
    struct Args {
        #[allow(dead_code)]
        workspace_id: uuid::Uuid,
    }

    #[test]
    fn a_malformed_or_missing_argument_is_refused_by_name() {
        for args in [json!({ "workspace_id": "not-a-uuid" }), json!({})] {
            let err = parse_args::<Args>(&args).unwrap_err();
            assert!(err.to_string().contains("workspace_id"), "{err}");
        }
    }

    #[test]
    fn every_tool_that_declares_no_arguments_refuses_one_by_name() {
        let mut checked = 0;
        for tool in super::catalog() {
            let name = tool["name"].as_str().expect("a tool name");
            let takes_none = tool["inputSchema"]["properties"]
                .as_object()
                .is_none_or(|properties| properties.is_empty());
            let refused =
                refuse_arguments_to_a_tool_that_takes_none(name, &json!({ "token_id": "x" }));
            if takes_none {
                let err = refused.expect_err(name).to_string();
                assert!(err.contains("`token_id`") && err.contains(name), "{err}");
                for none in [json!({}), serde_json::Value::Null] {
                    assert!(
                        refuse_arguments_to_a_tool_that_takes_none(name, &none).is_ok(),
                        "{name}"
                    );
                }
                checked += 1;
            } else {
                assert!(
                    refused.is_ok(),
                    "{name} declares arguments; its own struct judges them"
                );
            }
        }
        assert!(
            checked >= 10,
            "only {checked} tools take no arguments; the catalog moved"
        );
    }
}

pub fn is_read_only(name: &str) -> bool {
    READ_ONLY_TOOLS.binary_search(&name).is_ok()
}

/// The capability a registrant needs, beyond `workspace:write`, to bind a
/// slash command to `tool`. A slash command runs its tool as whoever types
/// it, so a writer who bound `/help` to a destructive tool could wait for an
/// admin to type it (RED-004). A read-only tool needs nothing more; any other
/// takes `channel:admin`.
pub fn slash_target_capability(tool: &str) -> Option<&'static str> {
    (!is_read_only(tool)).then_some(maidan_auth::capability::CHANNEL_ADMIN)
}

/// The tools that need `member:self`: writes to the caller's own personal
/// state (MCP-002). Kept beside [`required_capability`], which must agree.
pub const MEMBER_SELF_TOOLS: &[&str] = &[
    "mark_inbox_read",
    "mark_notification_read",
    "mark_all_notifications_read",
    "snooze_notification",
    "set_notification_pref",
    "set_delivery_mode",
    "set_member_email",
    "delete_member_email",
    "follow_channel",
    "unfollow_channel",
    "follow_thread",
    "unfollow_thread",
    "follow_member",
    "unfollow_member",
    "mute_thread",
    "unmute_thread",
    "mute_channel",
    "unmute_channel",
];

pub fn required_capability(name: &str) -> Result<&'static str, McpError> {
    match name {
        "list_channels"
        | "list_threads"
        | "list_messages"
        | "list_dm_conversations"
        | "list_group_dms"
        | "get_group_dm"
        | "list_reactions"
        | "list_pins"
        | "get_artifact_metadata"
        | "get_artifact"
        | "get_thread_context"
        | "get_tool_transcript"
        | "get_workspace_context"
        | "get_approval_gate"
        | "list_mentions"
        | "get_inbox"
        | "wait_for_mention"
        | "wait_for_ready"
        | "wait_for_claim_expired"
        | "wait_for_claim_failed"
        | "wait_for_landed"
        | "wait_for_event"
        | "wait_for_blocked_resolved"
        | "get_queue_depth"
        | "get_channel_occupancy"
        | "get_workspace_queue_depth"
        | "get_workspace_occupancy"
        | "get_swarm_metrics"
        | "get_credits"
        | "top_up_credits"
        | "find_thread"
        | "find_threads"
        | "get_run_occupancy"
        | "get_thread_lineage"
        | "list_run_threads"
        | "list_assigned_threads"
        | "get_wip_limit"
        | "get_delegation_policy"
        | "get_retention_policy"
        | "get_spawn_budget"
        | "get_member_wip"
        | "get_member_occupancy"
        | "list_unclaimable"
        | "list_blocked_threads"
        | "get_thread_block"
        | "get_thread_status"
        | "get_thread_version"
        | "get_review_packet"
        | "list_thread_artifacts"
        | "list_evidence_records"
        | "get_wait"
        | "get_priority"
        | "list_thread_dependencies"
        | "list_thread_dependents"
        | "list_task_schedules"
        | "list_recipes"
        | "list_member_skills"
        | "list_thread_required_skills"
        | "list_thread_results"
        | "get_thread_result"
        | "list_active_claims"
        | "list_result_deliveries"
        | "get_thread_steer"
        | "wait_for_result"
        | "get_dependency_results"
        | "get_waiting_inbox"
        | "list_notifications"
        | "list_notifications_grouped"
        | "list_buried_decisions"
        | "get_manager_digest"
        | "get_unread_count"
        | "wait_for_notification"
        | "list_notification_prefs"
        | "get_delivery_mode"
        | "get_member_email"
        | "list_channel_follows"
        | "list_thread_follows"
        | "list_member_follows"
        | "list_child_threads"
        | "list_recently_active_threads"
        | "get_thread_budget"
        | "usage_rollup"
        | "list_dlq"
        | "get_glossary_term"
        | "list_glossary_terms"
        | "list_slack_channel_links"
        | "list_github_issue_links"
        | "get_memory_block"
        | "list_memory_blocks"
        | "list_thread_memory_blocks"
        | "wait_for_memory_block"
        | "get_review_status"
        | "get_review_requirement"
        | "list_reviewers"
        | "list_reviews"
        | "list_review_history"
        | "get_land_gate"
        | "list_land_gate_history"
        | "whoami"
        | "get_log_snapshot"
        | "verify_event_chain"
        | "list_tombstones"
        | "list_message_backlinks"
        | "list_message_edits"
        | "list_votes"
        | "list_members"
        | "get_member"
        | "get_kind_census"
        | "list_capability_sets"
        | "get_room"
        | "parse_maidan_uri"
        | "attenuate_token"
        | "rotate_token" => Ok(WORKSPACE_READ),
        "delegate_token" => Ok(WORKSPACE_READ),
        // The holder's own personal state (MCP-002). Self-scoped to the
        // token's member by `enforce_member_self_scope`; `member:self` says
        // the token may write it at all, so a bare `workspace:read` token
        // can't. `MEMBER_SELF_TOOLS` is the same list.
        "mark_inbox_read"
        | "mark_notification_read"
        | "mark_all_notifications_read"
        | "snooze_notification"
        | "set_notification_pref"
        | "set_delivery_mode"
        | "set_member_email"
        | "delete_member_email"
        | "follow_channel"
        | "unfollow_channel"
        | "follow_thread"
        | "unfollow_thread"
        | "follow_member"
        | "unfollow_member"
        | "mute_thread"
        | "unmute_thread"
        | "mute_channel"
        | "unmute_channel" => Ok(MEMBER_SELF),
        "create_delegation_grant"
        | "list_delegation_grants"
        | "revoke_delegation_grant"
        | "set_delegation_policy"
        | "set_retention_policy" => Ok(TOKEN_ADMIN),
        // Both reach people: a conversation they are put in, or a gate that
        // asks one of them to decide. A read-only token does neither (MCP-003).
        "open_dm_conversation"
        | "open_group_dm"
        | "request_approval"
        | "post_dm_message"
        | "post_group_dm_message"
        | "post_message"
        | "edit_message"
        | "tombstone_message" => Ok(MESSAGE_POST),
        "record_mention"
        | "cast_vote"
        | "retract_vote"
        | "add_reaction"
        | "remove_reaction"
        | "link_thread_artifact"
        | "unlink_thread_artifact"
        | "add_evidence_record"
        | "pin_message"
        | "unpin_message"
        | "add_reference"
        | "create_channel"
        | "create_thread"
        | "submit_plan"
        | "amend_plan"
        | "watch_pull"
        | "cancel_thread"
        | "create_task_schedule"
        | "create_recipe"
        | "instantiate_recipe"
        | "set_glossary_term"
        | "seed_from_message"
        | "set_wip_limit"
        | "set_spawn_budget"
        | "add_member_skill"
        | "create_memory_block"
        | "set_memory_block_value"
        | "attach_memory_block"
        | "detach_memory_block"
        | "replay_result_delivery"
        | "set_workspace_handle"
        | "remove_member_skill"
        | "delete_memory_block"
        | "delete_glossary_term"
        | "delete_recipe"
        | "set_task_schedule_active"
        | "delete_task_schedule"
        | "revoke_slash_command"
        | "revoke_fsm_hook"
        | "register_thread_pull" => Ok(WORKSPACE_WRITE),
        // The same as answering over REST. Accepting needs more, decided in
        // the tool: see `approval::approval_decide`.
        "approval_decide" => Ok(WORKSPACE_WRITE),
        "upload_artifact"
        | "begin_artifact_multipart"
        | "upload_artifact_multipart_part"
        | "complete_artifact_multipart"
        | "abort_artifact_multipart"
        | "snapshot_thread_context" => Ok(ARTIFACT_UPLOAD),
        "search_messages" => Ok(SEARCH_QUERY),
        "list_secrets" | "resolve_secret" => Ok(SECRET_READ),
        "create_secret" | "delete_secret" => Ok(SECRET_ADMIN),
        // `allow_secret_egress_host` also needs `secret:read`, checked in the tool.
        "list_secret_egress_hosts" | "allow_secret_egress_host" | "revoke_secret_egress_host" => {
            Ok(SECRET_ADMIN)
        }
        "freeze_member"
        | "unfreeze_member"
        | "list_frozen_members"
        | "create_share_ticket"
        | "list_share_tickets"
        | "revoke_share_ticket"
        | "export_workspace"
        | "verify_workspace_export"
        | "import_workspace"
        // The whole log as one chain, every private channel and DM included.
        | "catch_up_events" => Ok(TOKEN_ADMIN),
        "register_slash_command" => Ok(WORKSPACE_WRITE),
        "list_slash_commands" => Ok(WORKSPACE_READ),
        "list_references" => Ok(WORKSPACE_READ),
        // A hook acts on every matching transition in the workspace (SEC-013).
        "register_fsm_hook" => Ok(WORKSPACE_ADMIN),
        "list_fsm_hooks" => Ok(WORKSPACE_READ),
        "link_slack_channel"
        | "unlink_slack_channel"
        | "link_github_issue"
        | "unlink_github_issue" => Ok(WORKSPACE_WRITE),
        "add_channel_member" | "list_channel_members" | "remove_channel_member" => {
            Ok(maidan_auth::capability::CHANNEL_ADMIN)
        }
        "assign_thread"
        | "claim_thread"
        | "unassign_thread"
        | "claim_next_thread"
        | "claim_next_workspace_thread"
        | "renew_claim"
        | "acknowledge_claim"
        | "release_claim"
        | "release_own_claims"
        | "set_thread_footprint"
        | "set_thread_attributes"
        | "add_thread_dependency"
        | "add_thread_required_skill"
        | "set_thread_result"
        | "set_thread_lineage"
        | "set_thread_owner"
        | "rename_thread"
        | "set_thread_budget"
        | "update_thread_budget"
        | "report_usage"
        | "mark_unclaimable"
        | "mark_claimable"
        | "set_thread_block"
        | "clear_thread_block"
        | "declare_status"
        | "set_wait"
        | "cancel_wait"
        | "set_priority"
        | "set_review_requirement"
        | "add_reviewer"
        | "submit_review"
        | "set_thread_steer"
        | "transition_thread"
        | "close_abandoned_thread"
        | "set_land_gate"
        | "require_land_gate"
        | "remove_thread_dependency"
        | "remove_thread_required_skill"
        | "clear_thread_lineage"
        | "advise_land_gate" => Ok(maidan_auth::capability::THREAD_TRANSITION),
        // A gate ratchets: arming is `thread:transition`, removing it is
        // `channel:admin`. Clearing the row makes the close-gate vacuous, so a
        // clear is as powerful as a close — and `thread:transition` is what a
        // close needs and what `maidan.agent.worker` carries.
        "clear_land_gate"
        | "clear_review_requirement"
        | "remove_reviewer" => Ok(maidan_auth::capability::CHANNEL_ADMIN),
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
}

/// Personal state belongs to one member, and a token acts as one member.
///
/// Every member tool takes a caller-supplied `member_id`, and until this gate
/// existed none of them checked it: an ordinary `workspace:read` token could
/// read any member's inbox or rewrite any member's delivery address — including
/// a member in another workspace, since the channel gate below never looks at
/// `member_id` and so no workspace check ran either.
///
/// The rule is self-scoping: a token acts on the personal state of the member
/// it was minted for, with no override. An orchestrator acting for one of its
/// agents holds a delegated token for that agent, whose `member_id` *is* the
/// agent — so it passes as itself, and the delegate is durably recorded as the
/// actor on every use.
///
/// Enforced here rather than in the 25 handlers because a rule spread across 25
/// call sites is a rule the 26th tool forgets. Every member tool, present and
/// future, passes through this function.
///
/// **One refusal, deliberately.** Another member in your workspace and a member
/// in someone else's workspace both fail the same equality check and get the
/// same error. Anything more specific would confirm that a member id exists on
/// this instance.
fn enforce_member_self_scope(auth: &AuthContext, name: &str, args: &Value) -> Result<(), McpError> {
    if !MEMBER_SCOPED_TOOLS.contains(&name) {
        return Ok(());
    }
    let Some(claimed) = args
        .get("member_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<uuid::Uuid>().ok())
        .map(maidan_types::MemberId)
    else {
        // Absent or malformed: the handler's own decode error is clearer than
        // anything this gate could say.
        return Ok(());
    };

    if claimed == auth.member_id() {
        return Ok(());
    }
    Err(McpError::Forbidden("member_id is not yours".to_string()))
}

/// Tools whose `member_id` argument names whose personal state is being touched.
/// Adding a member tool without classifying it is caught by
/// `every_member_id_tool_is_classified`.
const MEMBER_SCOPED_TOOLS: &[&str] = &[
    "list_mentions",
    "get_inbox",
    "mark_inbox_read",
    "get_waiting_inbox",
    "list_notifications",
    "get_unread_count",
    "list_notifications_grouped",
    "list_buried_decisions",
    "mark_notification_read",
    "mark_all_notifications_read",
    "list_group_dms",
    "snooze_notification",
    "set_notification_pref",
    "list_notification_prefs",
    "set_delivery_mode",
    "get_delivery_mode",
    "set_member_email",
    "get_member_email",
    "delete_member_email",
    "follow_channel",
    "unfollow_channel",
    "list_channel_follows",
    "follow_thread",
    "unfollow_thread",
    "list_thread_follows",
    "unfollow_member",
    "list_member_follows",
    // `member_id` here is the *follower* — the one acting. `followed_member_id`
    // is the target and is deliberately not gated: following someone is not an
    // act on their state. Guard the actor, not the target.
    "follow_member",
    // Whose mentions / notifications the caller is parked on.
    "wait_for_mention",
    "wait_for_notification",
    // The digest *rolls up* other members, but `member_id` is the manager whose
    // digest it is — reading someone else's is reading their reports' state.
    // Cross-member rollup is the feature; cross-member access is not.
    "get_manager_digest",
    "list_dm_conversations",
    // Declared skills control which work the routing loop may assign. They are
    // personal capability state, not a workspace-wide directory entry.
    "list_member_skills",
];

/// Member tools whose `member_id` names *work* state rather than personal
/// state, and which are cross-member **by design**.
///
/// Occupancy, WIP, and assignment queues describe who holds which task. They
/// are team surfaces and already enforce workspace/channel access in their
/// handlers; self-scoping them would delete the feature rather than secure it.
///
/// Listed rather than omitted so that "not gated" is a decision with a reason
/// attached, and a new tool cannot land in the gap between the two lists.
/// The id guard (`id_scope`) refuses a `member_id` tool on none of these lists.
const MEMBER_WORK_STATE_TOOLS: &[&str] = &[
    "get_member_occupancy",
    "get_member_wip",
    "list_assigned_threads",
    "get_member",
    "usage_rollup",
];

/// Tools whose `member_id` is personal state *or* an administrative target
/// depending on another argument, so the decision is made in the handler where
/// that argument is visible. `add_member_skill`: a routing tag is the member's
/// own declaration and only they may set it; a governance skill is authority an
/// operator confers on someone else under `channel:admin`.
const MEMBER_ARGUMENT_SCOPED_TOOLS: &[&str] = &["add_member_skill", "remove_member_skill"];

/// Tools where `member_id` is the object of an administrative or routing
/// action, not the caller identity. Those arguments remain legitimate targets.
const MEMBER_TARGET_TOOLS: &[&str] = &[
    "add_channel_member",
    "remove_channel_member",
    "freeze_member",
    "unfreeze_member",
    "add_reviewer",
    "remove_reviewer",
    "record_mention",
];

/// A tool naming a workspace names the caller's own. Seven tools took a
/// `workspace_id` and never compared it with the token's (`list_channels`,
/// `get_workspace_context`, `open_dm_conversation` and others), so any
/// workspace's token read another's channel list or opened a DM inside it.
/// Found by `tenant_isolation_e2e`.
fn enforce_workspace_scope(auth: &AuthContext, args: &Value) -> Result<(), McpError> {
    let named = args
        .get("workspace_id")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<uuid::Uuid>().ok());
    match named {
        Some(id) => auth
            .ensure_workspace(maidan_types::WorkspaceId(id))
            .map_err(McpError::from),
        None => Ok(()),
    }
}

/// A `tools/call`'s tool name and arguments; absent arguments are an empty
/// object.
pub fn tool_call(params: &Value) -> Result<(&str, Value), McpError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::InvalidParams("missing tool name".into()))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    Ok((name, args))
}

/// The checks on a tool's raw arguments that need no store: a named workspace
/// is the caller's, and a member tool's `member_id` is the caller. They read
/// the arguments before any handler decodes them, so what they read and what
/// the handler decodes must agree. Public so that pair can be fuzzed
/// (`fuzz/fuzz_targets/mcp_request.rs`).
pub fn check_argument_scope(auth: &AuthContext, name: &str, args: &Value) -> Result<(), McpError> {
    enforce_workspace_scope(auth, args)?;
    enforce_member_self_scope(auth, name, args)
}

pub async fn dispatch(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    name: &str,
    args: &Value,
    call: &crate::call_context::CallContext,
) -> Result<Value, McpError> {
    refuse_arguments_to_a_tool_that_takes_none(name, args)?;
    check_argument_scope(auth, name, args)?;
    id_scope::enforce(server, auth, name, args).await?;
    // Admitted: the capability, the workspace's parking place and the
    // allowance were checked before dispatch, and the arguments and id
    // access just now. Only then does a wait stop holding the request's
    // in-flight permit (RED-008).
    if crate::parked_waits::parks(name) {
        crate::parked_waits::release_permit();
    }
    tool(server, auth, name, args, call)?.await
}

/// One tool's call, built but not yet polled.
type ToolFuture<'a> = futures::future::BoxFuture<'a, Result<Value, McpError>>;

/// The call of the tool `name`, each built in [`boxed`]. This is a plain
/// function, not an async one, because of how an unoptimized build lays out a
/// frame: every future a function can build gets a slot of its own, shared
/// with no other match arm. When the dispatch awaited each tool in place, its
/// frames held a slot for every tool's future, about 470 KiB on aarch64, under
/// whichever tool ran, and a debug server's `upload_artifact` on S3 overflowed
/// tokio's 2 MiB worker stack and aborted. Here the match holds a closure per
/// tool, and the frame is gone before the tool runs.
fn tool<'a>(
    server: &'a crate::server::McpServer,
    auth: &'a AuthContext,
    name: &str,
    args: &'a Value,
    call: &'a crate::call_context::CallContext,
) -> Result<ToolFuture<'a>, McpError> {
    let store = &server.store;
    let search = &server.search;
    let embedding_provider = &server.embedding_provider;
    Ok(match name {
        "list_channels" => boxed(move || channel::list_channels(store, auth, args)),
        "create_channel" => boxed(move || channel::create_channel(server, auth, args)),
        "add_channel_member" => boxed(move || channel::add_channel_member(store, auth, args)),
        "list_channel_members" => boxed(move || channel::list_channel_members(store, auth, args)),
        "remove_channel_member" => boxed(move || channel::remove_channel_member(store, auth, args)),
        "open_dm_conversation" => boxed(move || channel::open_dm_conversation(store, auth, args)),
        "list_dm_conversations" => boxed(move || channel::list_dm_conversations(store, args)),
        "post_dm_message" => boxed(move || message::post_dm_message(server, auth, args)),
        "open_group_dm" => boxed(move || channel::open_group_dm(store, auth, args)),
        "list_group_dms" => boxed(move || channel::list_group_dms(store, auth, args)),
        "get_group_dm" => boxed(move || channel::get_group_dm(store, auth, args)),
        "post_group_dm_message" => {
            boxed(move || message::post_group_dm_message(server, auth, args))
        }
        "list_threads" => boxed(move || thread::list_threads(store, args)),
        "find_threads" => boxed(move || thread::find_threads(store, auth, args)),
        "create_thread" => boxed(move || thread::create_thread(server, auth, args)),
        "list_child_threads" => boxed(move || thread::list_child_threads(store, args)),
        "list_recently_active_threads" => {
            boxed(move || thread::list_recently_active_threads(store, args))
        }
        "mute_thread" => boxed(move || thread::mute_thread(store, auth, args)),
        "unmute_thread" => boxed(move || thread::unmute_thread(store, auth, args)),
        "mute_channel" => boxed(move || channel::mute_channel(store, auth, args)),
        "unmute_channel" => boxed(move || channel::unmute_channel(store, auth, args)),
        "set_thread_budget" => boxed(move || budget::set_thread_budget(store, args)),
        "update_thread_budget" => boxed(move || budget::update_thread_budget(store, args)),
        "get_thread_budget" => boxed(move || budget::get_thread_budget(store, args)),
        "report_usage" => boxed(move || budget::report_usage(server, auth, args)),
        "usage_rollup" => boxed(move || budget::usage_rollup(server, auth, args)),
        "list_dlq" => boxed(move || budget::list_dlq(store, args)),
        "get_tool_transcript" => boxed(move || thread::get_tool_transcript(store, args)),
        "assign_thread" => boxed(move || thread::assign_thread(server, auth, args)),
        "claim_thread" => boxed(move || thread::claim_thread(server, auth, args)),
        "unassign_thread" => boxed(move || thread::unassign_thread(server, auth, args)),
        "transition_thread" => boxed(move || thread::transition_thread(server, auth, args)),
        "close_abandoned_thread" => {
            boxed(move || thread::close_abandoned_thread(server, auth, args))
        }
        "list_assigned_threads" => boxed(move || thread::list_assigned_threads(store, auth, args)),
        "set_wip_limit" => boxed(move || thread::set_wip_limit(store, auth, args)),
        "get_wip_limit" => boxed(move || thread::get_wip_limit(store, auth, args)),
        "set_spawn_budget" => boxed(move || spawn::set_spawn_budget(store, auth, args)),
        "get_spawn_budget" => boxed(move || spawn::get_spawn_budget(store, auth, args)),
        "get_member_wip" => boxed(move || thread::get_member_wip(store, auth, args)),
        "get_member_occupancy" => boxed(move || member::get_member_occupancy(server, auth, args)),
        "mark_unclaimable" => boxed(move || thread::mark_unclaimable(store, auth, args)),
        "cancel_thread" => boxed(move || thread::cancel_thread(server, auth, args)),
        "mark_claimable" => boxed(move || thread::mark_claimable(store, args)),
        "list_unclaimable" => boxed(move || thread::list_unclaimable(store, args)),
        "set_thread_block" => boxed(move || thread::set_thread_block(store, auth, args)),
        "get_thread_block" => boxed(move || thread::get_thread_block(store, args)),
        "clear_thread_block" => boxed(move || thread::clear_thread_block(server, auth, args)),
        "list_blocked_threads" => boxed(move || thread::list_blocked_threads(store, args)),
        "declare_status" => boxed(move || thread::declare_status(server, auth, args)),
        "get_thread_status" => boxed(move || thread::get_thread_status(store, args)),
        "get_thread_version" => boxed(move || thread::get_thread_version(store, args)),
        "get_review_packet" => boxed(move || thread::get_review_packet(store, args)),
        "list_thread_artifacts" => boxed(move || thread::list_thread_artifacts(store, args)),
        "link_thread_artifact" => boxed(move || thread::link_thread_artifact(store, auth, args)),
        "unlink_thread_artifact" => boxed(move || thread::unlink_thread_artifact(store, args)),
        "add_evidence_record" => boxed(move || thread::add_evidence_record(server, auth, args)),
        "list_evidence_records" => boxed(move || thread::list_evidence_records(store, args)),
        "set_wait" => boxed(move || thread::set_wait(store, auth, args)),
        "cancel_wait" => boxed(move || thread::cancel_wait(store, args)),
        "get_wait" => boxed(move || thread::get_wait(store, args)),
        "set_priority" => boxed(move || thread::set_priority(store, auth, args)),
        "get_priority" => boxed(move || thread::get_priority(store, args)),
        "claim_next_thread" => boxed(move || thread::claim_next_thread(server, auth, args)),
        "claim_next_workspace_thread" => {
            boxed(move || thread::claim_next_workspace_thread(server, auth, args))
        }
        "renew_claim" => boxed(move || thread::renew_claim(server, auth, args)),
        "acknowledge_claim" => boxed(move || thread::acknowledge_claim(server, auth, args)),
        "release_claim" => boxed(move || thread::release_claim(server, auth, args)),
        "release_own_claims" => boxed(move || thread::release_own_claims(server, auth, args)),
        "set_thread_footprint" => boxed(move || thread::set_thread_footprint(store, args)),
        "set_thread_attributes" => boxed(move || thread::set_thread_attributes(store, auth, args)),
        "submit_plan" => boxed(move || thread::submit_plan(server, auth, args)),
        "amend_plan" => boxed(move || thread::amend_plan(server, auth, args)),
        "watch_pull" => boxed(move || thread::watch_pull(server, auth, args)),
        "list_active_claims" => boxed(move || thread::list_active_claims(store, auth, args)),
        "add_thread_dependency" => boxed(move || thread::add_thread_dependency(store, auth, args)),
        "list_thread_dependencies" => boxed(move || thread::list_thread_dependencies(store, args)),
        "remove_thread_dependency" => boxed(move || thread::remove_thread_dependency(store, args)),
        "list_thread_dependents" => boxed(move || thread::list_thread_dependents(store, args)),
        "list_mentions" => boxed(move || member::list_mentions(store, args)),
        "get_inbox" => boxed(move || member::get_inbox(store, args)),
        "mark_inbox_read" => boxed(move || member::mark_inbox_read(store, args)),
        "wait_for_mention" => boxed(move || member::wait_for_mention(server, auth, args)),
        "get_waiting_inbox" => boxed(move || member::get_waiting_inbox(store, auth, args)),
        "list_notifications" => boxed(move || member::list_notifications(store, auth, args)),
        "get_unread_count" => boxed(move || member::get_unread_count(store, args)),
        "list_notifications_grouped" => {
            boxed(move || member::list_notifications_grouped(store, args))
        }
        "list_buried_decisions" => boxed(move || member::list_buried_decisions(store, args)),
        "get_manager_digest" => boxed(move || member::get_manager_digest(server, auth, args)),
        "mark_notification_read" => boxed(move || member::mark_notification_read(store, args)),
        "mark_all_notifications_read" => {
            boxed(move || member::mark_all_notifications_read(store, args))
        }
        "list_members" => boxed(move || member::list_members(store, auth, args)),
        "get_member" => boxed(move || member::get_member(store, auth, args)),
        "snooze_notification" => boxed(move || member::snooze_notification(store, args)),
        "wait_for_notification" => boxed(move || member::wait_for_notification(server, auth, args)),
        "set_notification_pref" => boxed(move || member::set_notification_pref(store, args)),
        "list_notification_prefs" => boxed(move || member::list_notification_prefs(store, args)),
        "set_delivery_mode" => boxed(move || member::set_delivery_mode(store, args)),
        "get_delivery_mode" => boxed(move || member::get_delivery_mode(store, args)),
        "set_member_email" => boxed(move || member::set_member_email(store, args)),
        "get_member_email" => boxed(move || member::get_member_email(store, args)),
        "delete_member_email" => boxed(move || member::delete_member_email(store, args)),
        "follow_channel" => boxed(move || member::follow_channel(store, args)),
        "unfollow_channel" => boxed(move || member::unfollow_channel(store, args)),
        "list_channel_follows" => boxed(move || member::list_channel_follows(store, args)),
        "follow_thread" => boxed(move || member::follow_thread(store, args)),
        "unfollow_thread" => boxed(move || member::unfollow_thread(store, args)),
        "list_thread_follows" => boxed(move || member::list_thread_follows(store, args)),
        "follow_member" => boxed(move || member::follow_member(server, auth, args)),
        "unfollow_member" => boxed(move || member::unfollow_member(store, args)),
        "list_member_follows" => boxed(move || member::list_member_follows(store, args)),
        "wait_for_ready" => boxed(move || thread::wait_for_ready(server, auth, args)),
        "wait_for_claim_expired" => {
            boxed(move || thread::wait_for_claim_expired(server, auth, args))
        }
        "wait_for_claim_failed" => boxed(move || thread::wait_for_claim_failed(server, auth, args)),
        "wait_for_blocked_resolved" => {
            boxed(move || thread::wait_for_blocked_resolved(server, auth, args))
        }
        "wait_for_landed" => boxed(move || thread::wait_for_landed(server, auth, args)),
        "wait_for_event" => boxed(move || event_wait::wait_for_event(server, auth, args)),
        "get_queue_depth" => boxed(move || thread::get_queue_depth(store, auth, args)),
        "get_channel_occupancy" => boxed(move || thread::get_channel_occupancy(store, auth, args)),
        "get_workspace_queue_depth" => {
            boxed(move || thread::get_workspace_queue_depth(store, auth, args))
        }
        "get_workspace_occupancy" => {
            boxed(move || thread::get_workspace_occupancy(store, auth, args))
        }
        "get_run_occupancy" => boxed(move || thread::get_run_occupancy(store, auth, args)),
        "register_thread_pull" => {
            boxed(move || change_pull::register_thread_pull(server, auth, args))
        }
        "get_swarm_metrics" => boxed(move || change_pull::get_swarm_metrics(server, auth, args)),
        "get_credits" => boxed(move || credits::get_credits(server, auth, args)),
        "top_up_credits" => boxed(move || credits::top_up_credits(server, auth, args)),
        "find_thread" => boxed(move || thread_lookup::find_thread(server, auth, args)),
        "set_thread_lineage" => boxed(move || thread::set_thread_lineage(store, args)),
        "get_thread_lineage" => boxed(move || thread::get_thread_lineage(store, args)),
        "clear_thread_lineage" => boxed(move || thread::clear_thread_lineage(store, args)),
        "list_run_threads" => boxed(move || thread::list_run_threads(store, auth, args)),
        "set_thread_result" => boxed(move || thread::set_thread_result(server, auth, args)),
        "get_thread_result" => boxed(move || thread::get_thread_result(store, args)),
        "list_thread_results" => boxed(move || thread::list_thread_results(store, auth, args)),
        "list_result_deliveries" => boxed(move || delivery::list_result_deliveries(store, args)),
        "replay_result_delivery" => {
            boxed(move || delivery::replay_result_delivery_tool(server, auth, args))
        }
        "set_thread_owner" => boxed(move || thread::set_thread_owner(store, args)),
        "rename_thread" => boxed(move || thread::rename_thread(store, args)),
        "set_thread_steer" => boxed(move || thread::set_thread_steer(server, auth, args)),
        "get_thread_steer" => boxed(move || thread::get_thread_steer(store, args)),
        "wait_for_result" => boxed(move || thread::wait_for_result(server, auth, args)),
        "get_dependency_results" => {
            boxed(move || thread::get_dependency_results(store, auth, args))
        }
        "create_task_schedule" => boxed(move || schedule::create_task_schedule(store, auth, args)),
        "list_task_schedules" => boxed(move || schedule::list_task_schedules(store, auth, args)),
        "set_task_schedule_active" => {
            boxed(move || schedule::set_task_schedule_active(store, auth, args))
        }
        "delete_task_schedule" => boxed(move || schedule::delete_task_schedule(store, auth, args)),
        "create_recipe" => boxed(move || recipe::create_recipe(store, auth, args)),
        "list_recipes" => boxed(move || recipe::list_recipes(store, auth, args)),
        "delete_recipe" => boxed(move || recipe::delete_recipe(store, auth, args)),
        "instantiate_recipe" => boxed(move || recipe::instantiate_recipe(server, auth, args)),
        "list_secrets" => boxed(move || secret::list_secrets(store, auth, args)),
        "resolve_secret" => boxed(move || secret::resolve_secret(server, auth, args)),
        "create_secret" => boxed(move || secret::create_secret(server, auth, args)),
        "delete_secret" => boxed(move || secret::delete_secret(store, auth, args)),
        "list_secret_egress_hosts" => {
            boxed(move || secret::list_secret_egress_hosts(store, auth, args))
        }
        "allow_secret_egress_host" => {
            boxed(move || secret::allow_secret_egress_host(server, auth, args))
        }
        "revoke_secret_egress_host" => {
            boxed(move || secret::revoke_secret_egress_host(store, auth, args))
        }
        "freeze_member" => boxed(move || freeze::freeze_member(server, auth, args)),
        "unfreeze_member" => boxed(move || freeze::unfreeze_member(server, auth, args)),
        "list_frozen_members" => boxed(move || freeze::list_frozen_members(store, auth, args)),
        "create_share_ticket" => boxed(move || share::create_share_ticket(server, auth, args)),
        "list_share_tickets" => boxed(move || share::list_share_tickets(server, auth, args)),
        "revoke_share_ticket" => boxed(move || share::revoke_share_ticket(server, auth, args)),
        "export_workspace" => boxed(move || export::export_workspace(server, auth, args)),
        "verify_workspace_export" => {
            boxed(move || async move { export::verify_workspace_export(server, args) })
        }
        "import_workspace" => boxed(move || export::import_workspace(server, auth, args)),
        "get_log_snapshot" => boxed(move || event_log::get_log_snapshot(store, auth, args)),
        "catch_up_events" => boxed(move || event_log::catch_up_events(store, auth, args)),
        "verify_event_chain" => boxed(move || event_log::verify_event_chain(store, auth, args)),
        "list_tombstones" => boxed(move || explorer::list_tombstones(store, auth, args)),
        "list_message_backlinks" => {
            boxed(move || explorer::list_message_backlinks(store, auth, args))
        }
        "get_kind_census" => boxed(move || explorer::get_kind_census(store, auth, args)),
        "create_memory_block" => {
            boxed(move || memory_block::create_memory_block(store, auth, args))
        }
        "get_memory_block" => boxed(move || memory_block::get_memory_block(store, auth, args)),
        "list_memory_blocks" => boxed(move || memory_block::list_memory_blocks(store, auth, args)),
        "set_memory_block_value" => {
            boxed(move || memory_block::set_memory_block_value(server, auth, args))
        }
        "attach_memory_block" => {
            boxed(move || memory_block::attach_memory_block(store, auth, args))
        }
        "detach_memory_block" => {
            boxed(move || memory_block::detach_memory_block(store, auth, args))
        }
        "delete_memory_block" => {
            boxed(move || memory_block::delete_memory_block(store, auth, args))
        }
        "list_thread_memory_blocks" => {
            boxed(move || memory_block::list_thread_memory_blocks(store, auth, args))
        }
        "wait_for_memory_block" => {
            boxed(move || memory_block::wait_for_memory_block(server, auth, args))
        }
        "set_review_requirement" => {
            boxed(move || review::set_review_requirement(store, auth, args))
        }
        "add_reviewer" => boxed(move || review::add_reviewer(store, auth, args)),
        "submit_review" => boxed(move || review::submit_review(server, auth, args)),
        "get_review_status" => boxed(move || review::get_review_status(store, args)),
        "list_reviews" => boxed(move || review::list_reviews(store, args)),
        "list_review_history" => boxed(move || review::list_review_history(store, args)),
        "get_review_requirement" => boxed(move || review::get_review_requirement(store, args)),
        "clear_review_requirement" => {
            boxed(move || review::clear_review_requirement(store, auth, args))
        }
        "list_reviewers" => boxed(move || review::list_reviewers(store, args)),
        "remove_reviewer" => boxed(move || review::remove_reviewer(store, auth, args)),
        "set_land_gate" => boxed(move || land_gate::set_land_gate(store, auth, args)),
        "get_land_gate" => boxed(move || land_gate::get_land_gate(store, args)),
        "list_land_gate_history" => boxed(move || land_gate::list_land_gate_history(store, args)),
        "require_land_gate" => boxed(move || land_gate::require_land_gate(store, args)),
        "clear_land_gate" => boxed(move || land_gate::clear_land_gate(store, auth, args)),
        "advise_land_gate" => boxed(move || land_gate::advise_land_gate(server, store, args)),
        "set_glossary_term" => boxed(move || glossary::set_glossary_term(store, auth, args)),
        "get_glossary_term" => boxed(move || glossary::get_glossary_term(store, auth, args)),
        "list_glossary_terms" => boxed(move || glossary::list_glossary_terms(store, auth, args)),
        "delete_glossary_term" => boxed(move || glossary::delete_glossary_term(store, auth, args)),
        "add_member_skill" => boxed(move || skill::add_member_skill(store, auth, args)),
        "list_member_skills" => boxed(move || skill::list_member_skills(store, args)),
        "remove_member_skill" => boxed(move || skill::remove_member_skill(store, auth, args)),
        "add_thread_required_skill" => boxed(move || skill::add_thread_required_skill(store, args)),
        "list_thread_required_skills" => {
            boxed(move || skill::list_thread_required_skills(store, args))
        }
        "remove_thread_required_skill" => {
            boxed(move || skill::remove_thread_required_skill(store, args))
        }
        "list_messages" => boxed(move || message::list_messages(store, args)),
        "post_message" => boxed(move || message::post_message(server, auth, args)),
        "edit_message" => boxed(move || message::edit_message(server, auth, args)),
        "tombstone_message" => boxed(move || message::tombstone_message(server, auth, args)),
        "list_message_edits" => boxed(move || message::list_message_edits(store, args)),
        "seed_from_message" => boxed(move || seed::seed_from_message(server, auth, args)),
        "record_mention" => boxed(move || message::record_mention(server, args)),
        "cast_vote" => boxed(move || social::cast_vote(server, auth, args)),
        "retract_vote" => boxed(move || social::retract_vote(server, auth, args)),
        "add_reaction" => boxed(move || social::add_reaction(server, auth, args)),
        "remove_reaction" => boxed(move || social::remove_reaction(server, auth, args)),
        "list_reactions" => boxed(move || social::list_reactions(store, args)),
        "list_votes" => boxed(move || social::list_votes(store, args)),
        "pin_message" => boxed(move || social::pin_message(server, auth, args)),
        "unpin_message" => boxed(move || social::unpin_message(server, auth, args)),
        "list_pins" => boxed(move || social::list_pins(store, args)),
        "add_reference" => boxed(move || reference::add_reference(server, args)),
        "list_references" => boxed(move || reference::list_references(store, args)),
        "upload_artifact" => boxed(move || artifact::upload_artifact(server, auth, args)),
        "begin_artifact_multipart" => {
            boxed(move || artifact::begin_artifact_multipart(server, auth))
        }
        "upload_artifact_multipart_part" => {
            boxed(move || artifact::upload_artifact_multipart_part(server, auth, args))
        }
        "complete_artifact_multipart" => {
            boxed(move || artifact::complete_artifact_multipart(server, auth, args))
        }
        "abort_artifact_multipart" => {
            boxed(move || artifact::abort_artifact_multipart(server, auth, args))
        }
        "get_artifact_metadata" => {
            boxed(move || artifact::get_artifact_metadata(store, auth, args))
        }
        "get_artifact" => boxed(move || artifact::get_artifact(server, auth, args)),
        "search_messages" => boxed(move || {
            search::search_messages(
                search,
                embedding_provider,
                store,
                auth,
                args,
                server.usage_meters(),
            )
        }),
        "register_slash_command" => {
            boxed(move || automation::register_slash_command(store, auth, args))
        }
        "list_slash_commands" => boxed(move || automation::list_slash_commands(store, auth, args)),
        "register_fsm_hook" => boxed(move || automation::register_fsm_hook(store, auth, args)),
        "list_fsm_hooks" => boxed(move || automation::list_fsm_hooks(store, auth, args)),
        "revoke_slash_command" => {
            boxed(move || automation::revoke_slash_command(server, auth, args))
        }
        "revoke_fsm_hook" => boxed(move || automation::revoke_fsm_hook(server, auth, args)),
        "link_slack_channel" => boxed(move || projector::link_slack_channel(server, auth, args)),
        "list_slack_channel_links" => {
            boxed(move || projector::list_slack_channel_links(server, auth, args))
        }
        "unlink_slack_channel" => {
            boxed(move || projector::unlink_slack_channel(server, auth, args))
        }
        "link_github_issue" => boxed(move || projector::link_github_issue(server, auth, args)),
        "list_github_issue_links" => {
            boxed(move || projector::list_github_issue_links(server, auth, args))
        }
        "unlink_github_issue" => boxed(move || projector::unlink_github_issue(server, auth, args)),
        "get_thread_context" => boxed(move || async move {
            let rendered = crate::context::render_thread_context(store.as_ref(), args).await?;
            Ok(content_parts(&[&rendered.stable, &rendered.tail]))
        }),
        "snapshot_thread_context" => {
            boxed(move || snapshot::snapshot_thread_context(server, auth, args))
        }
        "get_workspace_context" => boxed(move || async move {
            // The thread page is filtered by the caller's access in the
            // query, so the pack needs no per-thread access check after it
            // is built. Filter before serialize so the bytes stay the
            // canonical struct order REST uses.
            let reader_id = auth.member_id();
            let packed =
                crate::context::load_workspace_context(store.as_ref(), reader_id, args).await?;
            Ok(content_json(&packed))
        }),
        "request_approval" => boxed(move || approval::request_approval(server, auth, args)),
        "get_approval_gate" => boxed(move || approval::get_approval_gate(server, auth, args, call)),
        "approval_decide" => boxed(move || approval::approval_decide(server, auth, args, call)),
        "whoami" => boxed(move || whoami::whoami(auth)),
        "list_capability_sets" => boxed(room::list_capability_sets),
        "parse_maidan_uri" => boxed(move || room::parse_maidan_uri(args)),
        "get_room" => boxed(move || room::get_room(store, auth, args)),
        "set_workspace_handle" => boxed(move || room::set_workspace_handle(store, auth, args)),
        "attenuate_token" => boxed(move || room::attenuate_token(store, auth, args)),
        "rotate_token" => boxed(move || room::rotate_token(store, auth, args)),
        "delegate_token" => boxed(move || room::delegate_token(store, auth, args)),
        "create_delegation_grant" => {
            boxed(move || room::create_delegation_grant(store, auth, args))
        }
        "set_delegation_policy" => boxed(move || room::set_delegation_policy(store, auth, args)),
        "get_delegation_policy" => boxed(move || room::get_delegation_policy(store, auth, args)),
        "set_retention_policy" => boxed(move || room::set_retention_policy(server, auth, args)),
        "get_retention_policy" => boxed(move || room::get_retention_policy(server, auth, args)),
        "list_delegation_grants" => boxed(move || room::list_delegation_grants(store, auth, args)),
        "revoke_delegation_grant" => {
            boxed(move || room::revoke_delegation_grant(store, auth, args))
        }
        other => return Err(McpError::MethodNotFound(format!("tools/{other}"))),
    })
}

/// Build one tool's future in a frame of its own, which [`tool`]'s match
/// would otherwise have to hold a slot for.
fn boxed<'a, F, Fut>(build: F) -> ToolFuture<'a>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, McpError>> + Send + 'a,
{
    Box::pin(build())
}

/// The workspace a thread belongs to, which an audit row about it is stamped
/// with. The pre-dispatch gate has already held it to the caller's workspace.
pub(super) async fn thread_workspace(
    store: &dyn maidan_store::Store,
    thread_id: maidan_types::ThreadId,
) -> Result<maidan_types::WorkspaceId, McpError> {
    let thread = store.get_thread(thread_id).await?;
    Ok(store.get_channel(thread.channel_id).await?.workspace_id)
}

/// The member a tool call names by id, if it is one of the caller's
/// workspace. Another workspace's member is not found, as an id that names no
/// member is: a different answer told the caller the id was a member
/// somewhere.
pub(super) async fn requested_member(
    store: &dyn maidan_store::Store,
    auth: &AuthContext,
    id: maidan_types::MemberId,
) -> Result<maidan_types::Member, McpError> {
    Ok(store.get_member_in(auth.workspace_id(), id).await?)
}

/// Wrap a JSON payload in MCP's `content[]` envelope. The MCP spec
/// requires tool results to be an array of content parts; for now we
/// always return a single `text` part with the JSON-stringified value.
pub(super) fn content_json<T: serde::Serialize>(value: &T) -> Value {
    let body = serde_json::to_string(value).unwrap_or_else(|_| "null".into());
    json!({
        "content": [
            { "type": "text", "text": body }
        ],
        "isError": false
    })
}

/// Two or more text parts. `get_thread_context` uses this so the stable prefix
/// and the volatile tail are separate strings a cache can pin independently.
pub(super) fn content_parts(parts: &[&str]) -> Value {
    let content: Vec<Value> = parts
        .iter()
        .map(|text| json!({ "type": "text", "text": text }))
        .collect();
    json!({
        "content": content,
        "isError": false
    })
}

#[cfg(test)]
mod catalog_filter_tests {
    use super::*;
    use maidan_types::{ApiTokenId, MemberId, WorkspaceId};

    fn tool_names(tools: &[Value]) -> Vec<String> {
        tools
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect()
    }

    #[test]
    fn a_read_only_token_sees_only_read_tools() {
        let auth = AuthContext::from_token(
            ApiTokenId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
            WorkspaceId(uuid::Uuid::new_v4()),
            vec![WORKSPACE_READ.to_string()],
        );
        let names = tool_names(&catalog_for(&auth));
        // A workspace:read tool is present; write / search / artifact tools are not.
        assert!(names.contains(&"list_threads".to_string()));
        assert!(!names.contains(&"post_message".to_string())); // message:post
        assert!(!names.contains(&"search_messages".to_string())); // search:query
        assert!(!names.contains(&"add_reaction".to_string())); // workspace:write
                                                               // Every surfaced tool really does require workspace:read.
        for name in &names {
            assert_eq!(required_capability(name).unwrap(), WORKSPACE_READ, "{name}");
        }
    }

    #[test]
    fn personal_state_writes_need_member_self_not_workspace_read() {
        assert_eq!(MEMBER_SELF_TOOLS.len(), 18);
        for name in MEMBER_SELF_TOOLS {
            assert_eq!(required_capability(name).unwrap(), MEMBER_SELF, "{name}");
            assert!(!is_read_only(name), "{name} writes");
        }
        let member = MemberId(uuid::Uuid::new_v4());
        let workspace = WorkspaceId(uuid::Uuid::new_v4());
        let read_only = AuthContext::from_token(
            ApiTokenId(uuid::Uuid::new_v4()),
            member,
            workspace,
            vec![WORKSPACE_READ.to_string()],
        );
        let names = tool_names(&catalog_for(&read_only));
        for name in MEMBER_SELF_TOOLS {
            assert!(
                !names.contains(&name.to_string()),
                "{name} offered to workspace:read"
            );
        }
        let personal = AuthContext::from_token(
            ApiTokenId(uuid::Uuid::new_v4()),
            member,
            workspace,
            vec![WORKSPACE_READ.to_string(), MEMBER_SELF.to_string()],
        );
        let names = tool_names(&catalog_for(&personal));
        for name in MEMBER_SELF_TOOLS {
            assert!(
                names.contains(&name.to_string()),
                "{name} hidden from member:self"
            );
        }
        let worker = maidan_auth::expand_set(maidan_auth::AGENT_WORKER).unwrap();
        assert!(
            worker.iter().any(|c| c == MEMBER_SELF),
            "the worker preset keeps them"
        );
        assert!(maidan_auth::capability::default_minted()
            .iter()
            .any(|c| c == MEMBER_SELF));
    }

    #[test]
    fn a_missing_capability_hides_its_tools() {
        let auth = AuthContext::from_token(
            ApiTokenId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
            WorkspaceId(uuid::Uuid::new_v4()),
            vec![SEARCH_QUERY.to_string()],
        );
        let names = tool_names(&catalog_for(&auth));
        assert_eq!(names, vec!["search_messages".to_string()]);
    }
}

#[cfg(test)]
mod self_scope_tests {
    use super::*;

    /// The gate is a list, and a list drifts. Classify every MCP schema that
    /// accepts `member_id`, regardless of which implementation module owns its
    /// dispatch arm. This catches personal-state tools such as member skills,
    /// which live outside `member.rs` and escaped the old source-text heuristic.
    #[test]
    fn every_member_id_tool_is_classified() {
        let declared: Vec<String> = catalog()
            .into_iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?;
                declared_arguments(name)?
                    .contains("member_id")
                    .then(|| name.to_owned())
            })
            .collect();

        assert!(
            declared.len() > 20,
            "expected broad member_id schema coverage; found {} — the catalog parse broke, \
             not the gate",
            declared.len()
        );

        let missing: Vec<&str> = declared
            .iter()
            .map(String::as_str)
            .filter(|name| {
                !MEMBER_SCOPED_TOOLS.contains(name)
                    && !MEMBER_WORK_STATE_TOOLS.contains(name)
                    && !MEMBER_TARGET_TOOLS.contains(name)
                    && !MEMBER_ARGUMENT_SCOPED_TOOLS.contains(name)
            })
            .collect();
        assert!(
            missing.is_empty(),
            "member_id tools left unclassified: {missing:?}. Classify the argument as \
             personal state, team-visible work state, a target, or argument-dependent."
        );

        // No tool may be claimed by more than one semantic class.
        let classes = [
            MEMBER_SCOPED_TOOLS,
            MEMBER_WORK_STATE_TOOLS,
            MEMBER_TARGET_TOOLS,
            MEMBER_ARGUMENT_SCOPED_TOOLS,
        ];
        let both: Vec<&&str> = classes
            .iter()
            .flat_map(|class| class.iter())
            .filter(|name| classes.iter().filter(|class| class.contains(name)).count() > 1)
            .collect();
        assert!(
            both.is_empty(),
            "member_id tools listed in multiple semantic classes: {both:?}"
        );
    }

    /// The gate reads one argument: `member_id`. A tool listed as
    /// member-scoped whose schema calls that argument something else would
    /// sail through the gate — listed, and ungated. Nothing else would notice.
    #[test]
    fn every_scoped_tool_declares_the_argument_the_gate_reads() {
        let wrong: Vec<&str> = MEMBER_SCOPED_TOOLS
            .iter()
            .copied()
            .filter(|name| declared_arguments(name).is_none_or(|args| !args.contains("member_id")))
            .collect();
        assert!(
            wrong.is_empty(),
            "listed in MEMBER_SCOPED_TOOLS but their schema has no `member_id`: {wrong:?} — \
             the gate cannot find the argument it is meant to check"
        );
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;

    #[test]
    fn the_read_only_list_is_sorted_so_lookup_finds_every_entry() {
        let mut sorted = READ_ONLY_TOOLS.to_vec();
        sorted.sort_unstable();
        assert_eq!(READ_ONLY_TOOLS, sorted.as_slice());
        assert!(READ_ONLY_TOOLS.iter().all(|name| is_read_only(name)));
    }

    #[test]
    fn every_read_only_tool_is_a_real_tool() {
        let names: Vec<String> = catalog()
            .iter()
            .filter_map(|tool| tool.get("name").and_then(|n| n.as_str()).map(String::from))
            .collect();
        for name in READ_ONLY_TOOLS {
            assert!(
                names.iter().any(|n| n == name),
                "{name} is listed read-only but is not in the catalog"
            );
        }
    }

    #[test]
    fn taking_data_out_is_recorded_like_a_change() {
        for name in ["export_workspace", "resolve_secret"] {
            assert!(!is_read_only(name), "{name} must leave a record");
        }
    }
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    /// Every long-poll tool's own bound (five minutes) fits inside its deadline,
    /// so the deadline never cuts a legitimate wait short.
    #[test]
    fn a_long_poll_tool_outlasts_its_own_wait_bound() {
        let names: Vec<String> = catalog()
            .iter()
            .filter_map(|t| t["name"].as_str().map(String::from))
            .collect();
        for bulk in BULK_TOOLS {
            assert!(names.iter().any(|n| n == bulk), "{bulk} is not a tool");
        }
        for tool in catalog() {
            let name = tool["name"].as_str().unwrap_or_default();
            let limit = deadline(name);
            if name.starts_with("wait_for_") || BULK_TOOLS.contains(&name) {
                assert!(limit > std::time::Duration::from_secs(300), "{name}");
            } else {
                assert_eq!(limit, TOOL_DEADLINE, "{name}");
            }
        }
    }
}
