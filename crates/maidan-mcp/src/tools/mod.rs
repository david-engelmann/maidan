//! MCP tools backed by [`maidan_store::Store`]. Each tool has a JSON
//! schema (input shape) and a dispatcher that decodes args, calls the
//! store, and returns a JSON result.
//!
//! The per-tool handlers are organized by domain in the submodules
//! below; the three entry points (`required_capability`, `catalog`,
//! `dispatch`) and the shared [`content_json`] helper live here.

use maidan_auth::capability::{
    ARTIFACT_UPLOAD, MESSAGE_POST, SEARCH_QUERY, SECRET_READ, TOKEN_ADMIN, WORKSPACE_READ,
    WORKSPACE_WRITE,
};
use maidan_auth::AuthContext;
use serde_json::{json, Value};

use crate::error::McpError;

mod approval;
mod artifact;
mod automation;
mod budget;
mod catalog;
mod channel;
mod delivery;
mod event_log;
mod explorer;
mod export;
mod freeze;
mod glossary;
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
mod whoami;

pub use catalog::{catalog, declared_arguments};

/// The tool catalog filtered to the tools the caller may invoke. Bypass callers
/// (auth disabled) see everything; otherwise a tool whose required capability
/// the caller lacks is omitted, so a capability-scoped agent gets a smaller,
/// relevant `tools/list` — fewer tokens and no tools it would only get 403s
/// from. The unfiltered [`catalog`] is unchanged (contract tests +
/// full-capability callers rely on it).
pub fn catalog_for(auth: &AuthContext) -> Vec<Value> {
    catalog()
        .into_iter()
        .filter(|tool| {
            if auth.bypass {
                return true;
            }
            tool.get("name")
                .and_then(|n| n.as_str())
                .and_then(|name| required_capability(name).ok())
                .is_some_and(|cap| auth.has_capability(cap))
        })
        .collect()
}

/// Tools that change nothing. Every other tool is treated as a change and, if
/// it leaves no record of its own, gets one written for it by
/// [`crate::server::McpServer`]. Listing a tool here is a claim that it writes
/// nothing: getting it wrong in this direction loses a record, so the list is
/// explicit rather than a prefix rule a future `get_or_create_*` would satisfy.
/// Exporting a workspace and resolving a secret are reads that are
/// deliberately absent — who took the data is part of the record.
pub const READ_ONLY_TOOLS: &[&str] = &[
    "catch_up_events",
    "get_approval_gate",
    "get_artifact_metadata",
    "get_channel_occupancy",
    "get_delegation_policy",
    "get_delivery_mode",
    "get_dependency_results",
    "get_glossary_term",
    "get_inbox",
    "get_kind_census",
    "get_land_gate",
    "get_log_snapshot",
    "get_manager_digest",
    "get_member_email",
    "get_member_occupancy",
    "get_member_wip",
    "get_memory_block",
    "get_priority",
    "get_queue_depth",
    "get_review_status",
    "get_room",
    "get_run_occupancy",
    "get_spawn_budget",
    "get_thread_block",
    "get_thread_budget",
    "get_thread_context",
    "get_thread_lineage",
    "get_thread_result",
    "get_thread_steer",
    "get_tool_transcript",
    "get_unread_count",
    "get_wait",
    "get_waiting_inbox",
    "get_wip_limit",
    "get_workspace_context",
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
    "list_frozen_members",
    "list_fsm_hooks",
    "list_github_issue_links",
    "list_glossary_terms",
    "list_member_follows",
    "list_member_skills",
    "list_memory_blocks",
    "list_mentions",
    "list_message_backlinks",
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
    "list_reviews",
    "list_run_threads",
    "list_secrets",
    "list_share_tickets",
    "list_slack_channel_links",
    "list_slash_commands",
    "list_task_schedules",
    "list_thread_dependencies",
    "list_thread_follows",
    "list_thread_memory_blocks",
    "list_thread_required_skills",
    "list_thread_results",
    "list_threads",
    "list_tombstones",
    "list_unclaimable",
    "parse_maidan_uri",
    "search_messages",
    "verify_event_chain",
    "verify_workspace_export",
    "wait_for_claim_expired",
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

pub fn is_read_only(name: &str) -> bool {
    READ_ONLY_TOOLS.binary_search(&name).is_ok()
}

pub fn required_capability(name: &str) -> Result<&'static str, McpError> {
    match name {
        "list_channels"
        | "list_threads"
        | "list_messages"
        | "list_dm_conversations"
        | "list_reactions"
        | "list_pins"
        | "get_artifact_metadata"
        | "get_thread_context"
        | "get_tool_transcript"
        | "get_workspace_context"
        | "request_approval"
        | "get_approval_gate"
        | "list_mentions"
        | "get_inbox"
        | "mark_inbox_read"
        | "wait_for_mention"
        | "wait_for_ready"
        | "wait_for_claim_expired"
        | "wait_for_landed"
        | "get_queue_depth"
        | "get_channel_occupancy"
        | "get_run_occupancy"
        | "get_thread_lineage"
        | "list_run_threads"
        | "list_assigned_threads"
        | "get_wip_limit"
        | "get_delegation_policy"
        | "get_spawn_budget"
        | "get_member_wip"
        | "get_member_occupancy"
        | "list_unclaimable"
        | "list_blocked_threads"
        | "get_thread_block"
        | "get_wait"
        | "get_priority"
        | "list_thread_dependencies"
        | "list_task_schedules"
        | "list_recipes"
        | "list_member_skills"
        | "list_thread_required_skills"
        | "list_thread_results"
        | "get_thread_result"
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
        | "mark_notification_read"
        | "snooze_notification"
        | "wait_for_notification"
        | "list_notification_prefs"
        | "set_notification_pref"
        | "set_delivery_mode"
        | "get_delivery_mode"
        | "set_member_email"
        | "get_member_email"
        | "delete_member_email"
        | "follow_channel"
        | "unfollow_channel"
        | "list_channel_follows"
        | "follow_thread"
        | "unfollow_thread"
        | "list_thread_follows"
        | "follow_member"
        | "unfollow_member"
        | "list_member_follows"
        | "list_child_threads"
        | "list_recently_active_threads"
        | "mute_thread"
        | "unmute_thread"
        | "mute_channel"
        | "unmute_channel"
        | "get_thread_budget"
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
        | "list_reviews"
        | "get_land_gate"
        | "whoami"
        | "get_log_snapshot"
        | "catch_up_events"
        | "verify_event_chain"
        | "list_tombstones"
        | "list_message_backlinks"
        | "get_kind_census"
        | "list_capability_sets"
        | "get_room"
        | "parse_maidan_uri"
        | "attenuate_token" => Ok(WORKSPACE_READ),
        "delegate_token" => Ok(WORKSPACE_READ),
        "create_delegation_grant"
        | "list_delegation_grants"
        | "revoke_delegation_grant"
        | "set_delegation_policy" => Ok(TOKEN_ADMIN),
        "open_dm_conversation" | "post_dm_message" | "post_message" | "edit_message" => {
            Ok(MESSAGE_POST)
        }
        "record_mention"
        | "cast_vote"
        | "add_reaction"
        | "remove_reaction"
        | "pin_message"
        | "unpin_message"
        | "add_reference"
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
        | "set_workspace_handle" => Ok(WORKSPACE_WRITE),
        "upload_artifact"
        | "begin_artifact_multipart"
        | "upload_artifact_multipart_part"
        | "complete_artifact_multipart"
        | "abort_artifact_multipart"
        | "snapshot_thread_context" => Ok(ARTIFACT_UPLOAD),
        "search_messages" => Ok(SEARCH_QUERY),
        "list_secrets" | "resolve_secret" => Ok(SECRET_READ),
        "freeze_member"
        | "unfreeze_member"
        | "list_frozen_members"
        | "create_share_ticket"
        | "list_share_tickets"
        | "revoke_share_ticket"
        | "export_workspace"
        | "verify_workspace_export"
        | "import_workspace" => Ok(TOKEN_ADMIN),
        "register_slash_command" => Ok(WORKSPACE_WRITE),
        "list_slash_commands" => Ok(WORKSPACE_READ),
        "list_references" => Ok(WORKSPACE_READ),
        "register_fsm_hook" => Ok(WORKSPACE_WRITE),
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
        | "renew_claim"
        | "acknowledge_claim"
        | "release_claim"
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
        | "set_wait"
        | "cancel_wait"
        | "set_priority"
        | "set_review_requirement"
        | "add_reviewer"
        | "submit_review"
        | "set_thread_steer"
        | "transition_thread"
        | "set_land_gate"
        | "require_land_gate" => Ok(maidan_auth::capability::THREAD_TRANSITION),
        // A gate ratchets: arming is `thread:transition`, removing it is
        // `channel:admin`. Clearing the row makes the close-gate vacuous, so a
        // clear is as powerful as a close — and `thread:transition` is what a
        // close needs and what `maidan.agent.worker` carries.
        "clear_land_gate" => Ok(maidan_auth::capability::CHANNEL_ADMIN),
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
    if auth.bypass || !MEMBER_SCOPED_TOOLS.contains(&name) {
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

    if claimed == auth.member_id {
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
/// Referenced only by the guard below — the gate itself needs no list of what
/// it does *not* cover.
#[cfg(test)]
const MEMBER_WORK_STATE_TOOLS: &[&str] = &[
    "get_member_occupancy",
    "get_member_wip",
    "list_assigned_threads",
];

/// Tools where `member_id` is the object of an administrative or routing
/// action, not the caller identity. Those arguments remain legitimate targets.
/// Tools whose `member_id` is personal state *or* an administrative target
/// depending on another argument, so the decision is made in the handler where
/// that argument is visible. `add_member_skill`: a routing tag is the member's
/// own declaration and only they may set it; a governance skill is authority an
/// operator confers on someone else under `channel:admin`.
#[cfg(test)]
const MEMBER_ARGUMENT_SCOPED_TOOLS: &[&str] = &["add_member_skill"];

#[cfg(test)]
const MEMBER_TARGET_TOOLS: &[&str] = &[
    "add_channel_member",
    "remove_channel_member",
    "freeze_member",
    "unfreeze_member",
    "add_reviewer",
    "record_mention",
];

/// Tools whose `channel_id` is checked for channel access before dispatch.
const CHANNEL_SCOPED_TOOLS: &[&str] = &[
    "list_threads",
    "claim_next_thread",
    "wait_for_ready",
    "wait_for_claim_expired",
    "wait_for_landed",
    "get_queue_depth",
    "get_channel_occupancy",
    "list_recently_active_threads",
    "mute_channel",
    "unmute_channel",
    "list_dlq",
    "create_task_schedule",
    "create_recipe",
    "list_unclaimable",
    "list_blocked_threads",
    "follow_channel",
];

/// Tools whose optional `channel_id` and `thread_id` are both checked before
/// dispatch.
const CHANNEL_AND_THREAD_SCOPED_TOOLS: &[&str] = &["list_tombstones", "get_kind_census"];

/// Tools with a `channel_id` that is deliberately not checked for channel
/// access at dispatch, each for a reason its handler or the store carries.
/// Checked by `every_channel_id_tool_is_classified`.
///
/// - The membership tools: an admin manages a private channel it is not in, so
///   the handler checks the workspace instead (`channel::own_channel`).
/// - `create_share_ticket`: the store requires the channel, owner and creator
///   to be live in the ticket's workspace.
/// - `unfollow_channel`: deletes only the caller's own follow row.
/// - `seed_from_message`: scoped by its `message_id`.
/// - `search_messages`: filters its results by access.
#[cfg(test)]
const CHANNEL_HANDLER_SCOPED_TOOLS: &[&str] = &[
    "add_channel_member",
    "list_channel_members",
    "remove_channel_member",
    "create_share_ticket",
    "unfollow_channel",
    "seed_from_message",
    "search_messages",
];

/// Pre-dispatch per-channel authorization for point-access content tools.
/// Bypass callers pass through; DM tools rely on their own participant checks
/// (the `__dm__` channel is exempt in `ensure_*`); aggregate reads
/// (`list_channels` / `get_workspace_context` / `search_messages`) filter their
/// result sets separately. A tool whose id arg is absent/malformed is left to
/// its handler's own decode error.
async fn enforce_channel_access(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    name: &str,
    args: &Value,
) -> Result<(), McpError> {
    if auth.bypass {
        return Ok(());
    }
    let store = server.store.as_ref();
    let field = |key: &str| -> Option<uuid::Uuid> {
        args.get(key)
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse().ok())
    };
    match name {
        name if CHANNEL_SCOPED_TOOLS.contains(&name) => {
            // These tools' channel_id is optional; gate it only when present
            // so a caller can't long-poll a private channel they can't access.
            if let Some(id) = field("channel_id") {
                maidan_auth::ensure_channel_access(store, auth, maidan_types::ChannelId(id))
                    .await?;
            }
        }
        "list_messages"
        | "post_message"
        | "get_thread_context"
        | "snapshot_thread_context"
        | "get_tool_transcript"
        | "pin_message"
        | "unpin_message"
        | "list_pins"
        | "assign_thread"
        | "claim_thread"
        | "unassign_thread"
        | "renew_claim"
        | "acknowledge_claim"
        | "release_claim"
        | "add_thread_dependency"
        | "list_thread_dependencies"
        | "add_thread_required_skill"
        | "list_thread_required_skills"
        | "set_thread_result"
        | "set_thread_lineage"
        | "get_thread_result"
        | "get_thread_lineage"
        | "list_result_deliveries"
        | "replay_result_delivery"
        | "set_thread_owner"
        | "rename_thread"
        | "set_thread_steer"
        | "get_thread_steer"
        | "set_thread_budget"
        | "update_thread_budget"
        | "get_thread_budget"
        | "report_usage"
        | "list_child_threads"
        | "mute_thread"
        | "unmute_thread"
        | "wait_for_result"
        | "get_dependency_results"
        | "request_approval"
        | "mark_unclaimable"
        | "mark_claimable"
        | "set_thread_block"
        | "get_thread_block"
        | "clear_thread_block"
        | "set_wait"
        | "cancel_wait"
        | "get_wait"
        | "set_priority"
        | "get_priority"
        | "attach_memory_block"
        | "detach_memory_block"
        | "list_thread_memory_blocks"
        | "set_review_requirement"
        | "add_reviewer"
        | "submit_review"
        | "get_review_status"
        | "list_reviews"
        | "transition_thread"
        | "set_land_gate"
        | "get_land_gate"
        | "require_land_gate"
        | "clear_land_gate"
        | "follow_thread" => {
            if let Some(id) = field("thread_id") {
                maidan_auth::ensure_thread_access(store, auth, maidan_types::ThreadId(id)).await?;
            }
        }
        name if CHANNEL_AND_THREAD_SCOPED_TOOLS.contains(&name) => {
            if let Some(id) = field("channel_id") {
                maidan_auth::ensure_channel_access(store, auth, maidan_types::ChannelId(id))
                    .await?;
            }
            if let Some(id) = field("thread_id") {
                maidan_auth::ensure_thread_access(store, auth, maidan_types::ThreadId(id)).await?;
            }
        }
        "edit_message"
        | "record_mention"
        | "cast_vote"
        | "add_reaction"
        | "remove_reaction"
        | "list_reactions"
        | "seed_from_message"
        | "list_message_backlinks" => {
            if let Some(id) = field("message_id") {
                maidan_auth::ensure_message_access(store, auth, maidan_types::MessageId(id))
                    .await?;
            }
        }
        "add_reference" | "list_references" => {
            for (kind_key, id_key) in [("src_kind", "src_id"), ("dst_kind", "dst_id")] {
                if let (Some(kv), Some(id)) = (args.get(kind_key), field(id_key)) {
                    match serde_json::from_value::<maidan_types::RefSide>(kv.clone()) {
                        Ok(maidan_types::RefSide::Thread) => {
                            maidan_auth::ensure_thread_access(
                                store,
                                auth,
                                maidan_types::ThreadId(id),
                            )
                            .await?;
                        }
                        Ok(maidan_types::RefSide::Message) => {
                            maidan_auth::ensure_message_access(
                                store,
                                auth,
                                maidan_types::MessageId(id),
                            )
                            .await?;
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub async fn dispatch(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    name: &str,
    args: &Value,
) -> Result<Value, McpError> {
    enforce_member_self_scope(auth, name, args)?;
    enforce_channel_access(server, auth, name, args).await?;
    let store = &server.store;
    let artifacts = &server.artifacts;
    let search = &server.search;
    let embedding_provider = &server.embedding_provider;
    match name {
        "list_channels" => channel::list_channels(store, auth, args).await,
        "add_channel_member" => channel::add_channel_member(store, auth, args).await,
        "list_channel_members" => channel::list_channel_members(store, auth, args).await,
        "remove_channel_member" => channel::remove_channel_member(store, auth, args).await,
        "open_dm_conversation" => channel::open_dm_conversation(store, auth, args).await,
        "list_dm_conversations" => channel::list_dm_conversations(store, args).await,
        "post_dm_message" => message::post_dm_message(server, auth, args).await,
        "list_threads" => thread::list_threads(store, args).await,
        "list_child_threads" => thread::list_child_threads(store, args).await,
        "list_recently_active_threads" => thread::list_recently_active_threads(store, args).await,
        "mute_thread" => thread::mute_thread(store, auth, args).await,
        "unmute_thread" => thread::unmute_thread(store, auth, args).await,
        "mute_channel" => channel::mute_channel(store, auth, args).await,
        "unmute_channel" => channel::unmute_channel(store, auth, args).await,
        "set_thread_budget" => budget::set_thread_budget(store, args).await,
        "update_thread_budget" => budget::update_thread_budget(store, args).await,
        "get_thread_budget" => budget::get_thread_budget(store, args).await,
        "report_usage" => budget::report_usage(server, auth, args).await,
        "list_dlq" => budget::list_dlq(store, args).await,
        "get_tool_transcript" => thread::get_tool_transcript(store, args).await,
        "assign_thread" => thread::assign_thread(server, auth, args).await,
        "claim_thread" => thread::claim_thread(server, auth, args).await,
        "unassign_thread" => thread::unassign_thread(server, auth, args).await,
        "transition_thread" => thread::transition_thread(server, auth, args).await,
        "list_assigned_threads" => thread::list_assigned_threads(store, auth, args).await,
        "set_wip_limit" => thread::set_wip_limit(store, auth, args).await,
        "get_wip_limit" => thread::get_wip_limit(store, auth, args).await,
        "set_spawn_budget" => spawn::set_spawn_budget(store, auth, args).await,
        "get_spawn_budget" => spawn::get_spawn_budget(store, auth, args).await,
        "get_member_wip" => thread::get_member_wip(store, args).await,
        "get_member_occupancy" => member::get_member_occupancy(server, auth, args).await,
        "mark_unclaimable" => thread::mark_unclaimable(store, auth, args).await,
        "mark_claimable" => thread::mark_claimable(store, args).await,
        "list_unclaimable" => thread::list_unclaimable(store, args).await,
        "set_thread_block" => thread::set_thread_block(store, auth, args).await,
        "get_thread_block" => thread::get_thread_block(store, args).await,
        "clear_thread_block" => thread::clear_thread_block(server, auth, args).await,
        "list_blocked_threads" => thread::list_blocked_threads(store, args).await,
        "set_wait" => thread::set_wait(store, auth, args).await,
        "cancel_wait" => thread::cancel_wait(store, args).await,
        "get_wait" => thread::get_wait(store, args).await,
        "set_priority" => thread::set_priority(store, auth, args).await,
        "get_priority" => thread::get_priority(store, args).await,
        "claim_next_thread" => thread::claim_next_thread(server, auth, args).await,
        "renew_claim" => thread::renew_claim(server, auth, args).await,
        "acknowledge_claim" => thread::acknowledge_claim(server, auth, args).await,
        "release_claim" => thread::release_claim(server, auth, args).await,
        "add_thread_dependency" => thread::add_thread_dependency(store, auth, args).await,
        "list_thread_dependencies" => thread::list_thread_dependencies(store, args).await,
        "list_mentions" => member::list_mentions(store, args).await,
        "get_inbox" => member::get_inbox(store, args).await,
        "mark_inbox_read" => member::mark_inbox_read(store, args).await,
        "wait_for_mention" => member::wait_for_mention(server, auth, args).await,
        "get_waiting_inbox" => member::get_waiting_inbox(store, args).await,
        "list_notifications" => member::list_notifications(store, args).await,
        "get_unread_count" => member::get_unread_count(store, args).await,
        "list_notifications_grouped" => member::list_notifications_grouped(store, args).await,
        "list_buried_decisions" => member::list_buried_decisions(store, args).await,
        "get_manager_digest" => member::get_manager_digest(server, auth, args).await,
        "mark_notification_read" => member::mark_notification_read(store, args).await,
        "snooze_notification" => member::snooze_notification(store, args).await,
        "wait_for_notification" => member::wait_for_notification(server, auth, args).await,
        "set_notification_pref" => member::set_notification_pref(store, args).await,
        "list_notification_prefs" => member::list_notification_prefs(store, args).await,
        "set_delivery_mode" => member::set_delivery_mode(store, args).await,
        "get_delivery_mode" => member::get_delivery_mode(store, args).await,
        "set_member_email" => member::set_member_email(store, args).await,
        "get_member_email" => member::get_member_email(store, args).await,
        "delete_member_email" => member::delete_member_email(store, args).await,
        "follow_channel" => member::follow_channel(store, args).await,
        "unfollow_channel" => member::unfollow_channel(store, args).await,
        "list_channel_follows" => member::list_channel_follows(store, args).await,
        "follow_thread" => member::follow_thread(store, args).await,
        "unfollow_thread" => member::unfollow_thread(store, args).await,
        "list_thread_follows" => member::list_thread_follows(store, args).await,
        "follow_member" => member::follow_member(server, auth, args).await,
        "unfollow_member" => member::unfollow_member(store, args).await,
        "list_member_follows" => member::list_member_follows(store, args).await,
        "wait_for_ready" => thread::wait_for_ready(server, auth, args).await,
        "wait_for_claim_expired" => thread::wait_for_claim_expired(server, auth, args).await,
        "wait_for_landed" => thread::wait_for_landed(server, auth, args).await,
        "get_queue_depth" => thread::get_queue_depth(store, args).await,
        "get_channel_occupancy" => thread::get_channel_occupancy(store, args).await,
        "get_run_occupancy" => thread::get_run_occupancy(store, auth, args).await,
        "set_thread_lineage" => thread::set_thread_lineage(store, args).await,
        "get_thread_lineage" => thread::get_thread_lineage(store, args).await,
        "list_run_threads" => thread::list_run_threads(store, auth, args).await,
        "set_thread_result" => thread::set_thread_result(server, auth, args).await,
        "get_thread_result" => thread::get_thread_result(store, args).await,
        "list_thread_results" => thread::list_thread_results(store, auth, args).await,
        "list_result_deliveries" => delivery::list_result_deliveries(store, args).await,
        "replay_result_delivery" => delivery::replay_result_delivery_tool(store, auth, args).await,
        "set_thread_owner" => thread::set_thread_owner(store, args).await,
        "rename_thread" => thread::rename_thread(store, args).await,
        "set_thread_steer" => thread::set_thread_steer(server, auth, args).await,
        "get_thread_steer" => thread::get_thread_steer(store, args).await,
        "wait_for_result" => thread::wait_for_result(server, auth, args).await,
        "get_dependency_results" => thread::get_dependency_results(store, auth, args).await,
        "create_task_schedule" => schedule::create_task_schedule(store, auth, args).await,
        "list_task_schedules" => schedule::list_task_schedules(store, auth, args).await,
        "create_recipe" => recipe::create_recipe(store, auth, args).await,
        "list_recipes" => recipe::list_recipes(store, auth, args).await,
        "instantiate_recipe" => recipe::instantiate_recipe(server, auth, args).await,
        "list_secrets" => secret::list_secrets(store, auth, args).await,
        "resolve_secret" => secret::resolve_secret(server, auth, args).await,
        "freeze_member" => freeze::freeze_member(store, auth, args).await,
        "unfreeze_member" => freeze::unfreeze_member(store, auth, args).await,
        "list_frozen_members" => freeze::list_frozen_members(store, auth, args).await,
        "create_share_ticket" => share::create_share_ticket(server, auth, args).await,
        "list_share_tickets" => share::list_share_tickets(server, auth, args).await,
        "revoke_share_ticket" => share::revoke_share_ticket(server, auth, args).await,
        "export_workspace" => export::export_workspace(server, auth, args).await,
        "verify_workspace_export" => export::verify_workspace_export(server, args),
        "import_workspace" => export::import_workspace(server, auth, args).await,
        "get_log_snapshot" => event_log::get_log_snapshot(store, auth, args).await,
        "catch_up_events" => event_log::catch_up_events(store, auth, args).await,
        "verify_event_chain" => event_log::verify_event_chain(store, auth, args).await,
        "list_tombstones" => explorer::list_tombstones(store, auth, args).await,
        "list_message_backlinks" => explorer::list_message_backlinks(store, args).await,
        "get_kind_census" => explorer::get_kind_census(store, auth, args).await,
        "create_memory_block" => memory_block::create_memory_block(store, auth, args).await,
        "get_memory_block" => memory_block::get_memory_block(store, auth, args).await,
        "list_memory_blocks" => memory_block::list_memory_blocks(store, auth, args).await,
        "set_memory_block_value" => memory_block::set_memory_block_value(server, auth, args).await,
        "attach_memory_block" => memory_block::attach_memory_block(store, auth, args).await,
        "detach_memory_block" => memory_block::detach_memory_block(store, auth, args).await,
        "list_thread_memory_blocks" => {
            memory_block::list_thread_memory_blocks(store, auth, args).await
        }
        "wait_for_memory_block" => memory_block::wait_for_memory_block(server, auth, args).await,
        "set_review_requirement" => review::set_review_requirement(store, auth, args).await,
        "add_reviewer" => review::add_reviewer(store, auth, args).await,
        "submit_review" => review::submit_review(server, auth, args).await,
        "get_review_status" => review::get_review_status(store, args).await,
        "list_reviews" => review::list_reviews(store, args).await,
        "set_land_gate" => land_gate::set_land_gate(store, auth, args).await,
        "get_land_gate" => land_gate::get_land_gate(store, args).await,
        "require_land_gate" => land_gate::require_land_gate(store, args).await,
        "clear_land_gate" => land_gate::clear_land_gate(store, auth, args).await,
        "set_glossary_term" => glossary::set_glossary_term(store, auth, args).await,
        "get_glossary_term" => glossary::get_glossary_term(store, auth, args).await,
        "list_glossary_terms" => glossary::list_glossary_terms(store, auth, args).await,
        "add_member_skill" => skill::add_member_skill(store, auth, args).await,
        "list_member_skills" => skill::list_member_skills(store, args).await,
        "add_thread_required_skill" => skill::add_thread_required_skill(store, args).await,
        "list_thread_required_skills" => skill::list_thread_required_skills(store, args).await,
        "list_messages" => message::list_messages(store, args).await,
        "post_message" => message::post_message(server, auth, args).await,
        "edit_message" => message::edit_message(server, auth, args).await,
        "seed_from_message" => seed::seed_from_message(server, auth, args).await,
        "record_mention" => message::record_mention(server, args).await,
        "cast_vote" => social::cast_vote(server, auth, args).await,
        "add_reaction" => social::add_reaction(server, auth, args).await,
        "remove_reaction" => social::remove_reaction(server, auth, args).await,
        "list_reactions" => social::list_reactions(store, args).await,
        "pin_message" => social::pin_message(server, auth, args).await,
        "unpin_message" => social::unpin_message(server, auth, args).await,
        "list_pins" => social::list_pins(store, args).await,
        "add_reference" => reference::add_reference(server, args).await,
        "list_references" => reference::list_references(store, args).await,
        "upload_artifact" => artifact::upload_artifact(server, auth, args).await,
        "begin_artifact_multipart" => artifact::begin_artifact_multipart(artifacts).await,
        "upload_artifact_multipart_part" => {
            artifact::upload_artifact_multipart_part(artifacts, args).await
        }
        "complete_artifact_multipart" => {
            artifact::complete_artifact_multipart(server, auth, args).await
        }
        "abort_artifact_multipart" => artifact::abort_artifact_multipart(artifacts, args).await,
        "get_artifact_metadata" => artifact::get_artifact_metadata(store, auth, args).await,
        "search_messages" => {
            search::search_messages(search, embedding_provider, store, auth, args).await
        }
        "register_slash_command" => automation::register_slash_command(store, auth, args).await,
        "list_slash_commands" => automation::list_slash_commands(store, auth, args).await,
        "register_fsm_hook" => automation::register_fsm_hook(store, auth, args).await,
        "list_fsm_hooks" => automation::list_fsm_hooks(store, auth, args).await,
        "link_slack_channel" => projector::link_slack_channel(server, auth, args).await,
        "list_slack_channel_links" => projector::list_slack_channel_links(server, auth, args).await,
        "unlink_slack_channel" => projector::unlink_slack_channel(server, auth, args).await,
        "link_github_issue" => projector::link_github_issue(server, auth, args).await,
        "list_github_issue_links" => projector::list_github_issue_links(server, auth, args).await,
        "unlink_github_issue" => projector::unlink_github_issue(server, auth, args).await,
        "get_thread_context" => {
            let v = crate::context::get_thread_context(store.as_ref(), args).await?;
            Ok(content_json(&v))
        }
        "snapshot_thread_context" => snapshot::snapshot_thread_context(server, auth, args).await,
        "get_workspace_context" => {
            let mut v = crate::context::get_workspace_context(store.as_ref(), args).await?;
            // Drop packed threads in private channels the caller can't access,
            // caching the per-channel decision. Thread-keyed +
            // DM-participant-aware.
            if !auth.bypass {
                if let Some(threads) = v.get("threads").and_then(|t| t.as_array()) {
                    let mut decision: std::collections::HashMap<maidan_types::ThreadId, bool> =
                        std::collections::HashMap::new();
                    let mut kept = Vec::with_capacity(threads.len());
                    for t in threads {
                        let tid = t
                            .get("thread")
                            .and_then(|th| th.get("id"))
                            .and_then(|c| c.as_str())
                            .and_then(|s| s.parse::<uuid::Uuid>().ok())
                            .map(maidan_types::ThreadId);
                        let keep = match tid {
                            Some(id) => match decision.get(&id) {
                                Some(v) => *v,
                                None => {
                                    let ok =
                                        maidan_auth::can_access_thread(store.as_ref(), auth, id)
                                            .await?;
                                    decision.insert(id, ok);
                                    ok
                                }
                            },
                            None => true,
                        };
                        if keep {
                            kept.push(t.clone());
                        }
                    }
                    v["threads"] = Value::Array(kept);
                }
            }
            Ok(content_json(&v))
        }
        "request_approval" => approval::request_approval(server, auth, args).await,
        "get_approval_gate" => approval::get_approval_gate(server, auth, args).await,
        "whoami" => whoami::whoami(auth).await,
        "list_capability_sets" => room::list_capability_sets().await,
        "parse_maidan_uri" => room::parse_maidan_uri(args).await,
        "get_room" => room::get_room(store, auth, args).await,
        "set_workspace_handle" => room::set_workspace_handle(store, auth, args).await,
        "attenuate_token" => room::attenuate_token(store, auth, args).await,
        "delegate_token" => room::delegate_token(store, auth, args).await,
        "create_delegation_grant" => room::create_delegation_grant(store, auth, args).await,
        "set_delegation_policy" => room::set_delegation_policy(store, auth, args).await,
        "get_delegation_policy" => room::get_delegation_policy(store, auth, args).await,
        "list_delegation_grants" => room::list_delegation_grants(store, auth, args).await,
        "revoke_delegation_grant" => room::revoke_delegation_grant(store, auth, args).await,
        other => Err(McpError::MethodNotFound(format!("tools/{other}"))),
    }
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
    fn bypass_sees_the_whole_catalog() {
        let auth = AuthContext::bypass();
        assert_eq!(catalog_for(&auth).len(), catalog().len());
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

    /// A tool that names a channel is either checked for access to it before
    /// dispatch or listed with the reason it is not. The channel-membership
    /// tools were neither, and let one workspace's admin list and empty
    /// another's private channel.
    #[test]
    fn every_channel_id_tool_is_classified() {
        let declared: Vec<String> = catalog()
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?;
                declared_arguments(name)?
                    .contains("channel_id")
                    .then(|| name.to_owned())
            })
            .collect();
        assert!(
            declared.len() > 15,
            "expected broad channel_id schema coverage; found {}",
            declared.len()
        );
        let classes = [
            CHANNEL_SCOPED_TOOLS,
            CHANNEL_AND_THREAD_SCOPED_TOOLS,
            CHANNEL_HANDLER_SCOPED_TOOLS,
        ];
        for name in &declared {
            let claimed = classes
                .iter()
                .filter(|c| c.contains(&name.as_str()))
                .count();
            assert_eq!(
                claimed, 1,
                "channel_id tool {name} must be in exactly one class, found {claimed}"
            );
        }
        for class in classes {
            for name in class {
                assert!(
                    declared.iter().any(|d| d == name),
                    "{name} is classified but declares no channel_id"
                );
            }
        }
    }

    /// The gate reads one argument: `member_id`. A tool listed as
    /// member-scoped whose schema calls that argument something else would
    /// sail through the gate — listed, and ungated. Nothing else would notice.
    #[test]
    fn every_scoped_tool_declares_the_argument_the_gate_reads() {
        let catalog = include_str!("catalog.rs");
        let wrong: Vec<&str> = MEMBER_SCOPED_TOOLS
            .iter()
            .copied()
            .filter(|name| {
                let Some(start) = catalog.find(&format!("\"name\": \"{name}\"")) else {
                    return true;
                };
                let rest = &catalog[start + 8..];
                let end = rest.find("\"name\": \"").map_or(rest.len(), |i| i);
                !rest[..end].contains("\"member_id\"")
            })
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
