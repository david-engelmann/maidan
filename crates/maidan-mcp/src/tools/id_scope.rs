//! Which check binds each id a tool call names, decided per tool and per
//! argument, and refused when nobody decided (MCP-005).
//!
//! The pre-dispatch guard used to be a match on tool names whose default arm
//! did nothing: a new tool that took a `thread_id` and was never added to a
//! list ran with no access check, and nothing noticed. The guard now reads
//! each tool's id arguments from the tool's own schema (`catalog.json`), and
//! every one of them needs a row in [`ROWS`]: checked here, or exempt with the
//! reason that names what binds it instead. A call to a tool with an id
//! argument no row covers is refused before its handler runs, with auth
//! disabled too, and `every_id_argument_of_every_tool_is_classified` fails
//! first, in the tests.
//!
//! Rows are keyed by tool *and* argument, so giving an existing tool a new id
//! argument needs a decision as well.

use std::collections::HashMap;
use std::sync::LazyLock;

use maidan_auth::AuthContext;
use maidan_types::{ChannelId, MessageId, RefSide, ThreadId};
use serde_json::Value;

use crate::error::McpError;

/// What the guard does with one id argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IdCheck {
    /// The caller must be able to read the channel.
    Channel,
    /// The caller must be able to read the thread.
    Thread,
    /// The caller must be able to read the message.
    Message,
    /// One end of a reference. The sibling argument `kind_key` says what the
    /// id names: a thread or a message is checked as one; the handler owns
    /// any other kind.
    ReferenceEnd { kind_key: &'static str },
    /// The caller's own workspace, held by `enforce_workspace_scope` for every
    /// tool before this guard runs.
    Workspace,
    /// The caller themself, held by `enforce_member_self_scope`.
    Caller,
    /// Not checked here. The reason names what binds it instead.
    Exempt(&'static str),
}

struct Row {
    argument: &'static str,
    check: IdCheck,
    tools: &'static [&'static str],
}

const fn row(argument: &'static str, check: IdCheck, tools: &'static [&'static str]) -> Row {
    Row {
        argument,
        check,
        tools,
    }
}

/// Tools whose `channel_id` is checked for channel access before dispatch.
/// Optional in most of them, so it is checked only when present, which still
/// stops a caller long-polling a private channel it cannot read.
const CHANNEL_SCOPED_TOOLS: &[&str] = &[
    "list_threads",
    "create_thread",
    "claim_next_thread",
    "wait_for_ready",
    "wait_for_claim_expired",
    "wait_for_claim_failed",
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
    "watch_pull",
];

/// Tools whose optional `channel_id` and `thread_id` are both checked.
const CHANNEL_AND_THREAD_SCOPED_TOOLS: &[&str] = &[
    "list_tombstones",
    "get_kind_census",
    "wait_for_blocked_resolved",
    "wait_for_landed",
];

/// Tools whose `thread_id` is checked for thread access before dispatch.
const THREAD_SCOPED_TOOLS: &[&str] = &[
    "list_messages",
    "post_message",
    "get_thread_context",
    "snapshot_thread_context",
    "get_tool_transcript",
    "pin_message",
    "unpin_message",
    "list_pins",
    "assign_thread",
    "claim_thread",
    "unassign_thread",
    "renew_claim",
    "acknowledge_claim",
    "release_claim",
    "set_thread_footprint",
    "set_thread_attributes",
    "submit_plan",
    "amend_plan",
    "add_thread_dependency",
    "list_thread_dependencies",
    "remove_thread_dependency",
    "list_thread_dependents",
    "add_thread_required_skill",
    "list_thread_required_skills",
    "remove_thread_required_skill",
    "set_thread_result",
    "set_thread_lineage",
    "clear_thread_lineage",
    "get_thread_result",
    "get_thread_lineage",
    "list_result_deliveries",
    "replay_result_delivery",
    "set_thread_owner",
    "rename_thread",
    "set_thread_steer",
    "get_thread_steer",
    "set_thread_budget",
    "update_thread_budget",
    "get_thread_budget",
    "report_usage",
    "usage_rollup",
    "list_child_threads",
    "mute_thread",
    "unmute_thread",
    "wait_for_result",
    "get_dependency_results",
    "request_approval",
    "mark_unclaimable",
    "cancel_thread",
    "mark_claimable",
    "set_thread_block",
    "get_thread_block",
    "clear_thread_block",
    "declare_status",
    "get_thread_status",
    "get_thread_version",
    "get_review_packet",
    "list_thread_artifacts",
    "link_thread_artifact",
    "unlink_thread_artifact",
    "add_evidence_record",
    "list_evidence_records",
    "set_wait",
    "cancel_wait",
    "get_wait",
    "set_priority",
    "get_priority",
    "attach_memory_block",
    "detach_memory_block",
    "list_thread_memory_blocks",
    "set_review_requirement",
    "get_review_requirement",
    "clear_review_requirement",
    "list_reviewers",
    "remove_reviewer",
    "add_reviewer",
    "submit_review",
    "get_review_status",
    "list_reviews",
    "list_review_history",
    "transition_thread",
    "close_abandoned_thread",
    "set_land_gate",
    "get_land_gate",
    "list_land_gate_history",
    "require_land_gate",
    "clear_land_gate",
    "advise_land_gate",
    "register_thread_pull",
    "wait_for_event",
    "follow_thread",
];

/// Tools whose `message_id` is checked for message access before dispatch.
const MESSAGE_SCOPED_TOOLS: &[&str] = &[
    "edit_message",
    "record_mention",
    "cast_vote",
    "retract_vote",
    "add_reaction",
    "remove_reaction",
    "list_reactions",
    "seed_from_message",
    "list_message_backlinks",
    "tombstone_message",
    "list_message_edits",
    "list_votes",
];

const REFERENCE_TOOLS: &[&str] = &["add_reference", "list_references"];

const LOG_POSITION: &str =
    "a position in the event log that bounds a wait the other arguments scope; it reads nothing";
const PAGE_POSITION: &str =
    "a keyset position inside a page the other arguments scope; it reads nothing";
const FENCING_TOKEN: &str = "a fencing token compared with the guarded thread's live claim; \
     a foreign one only fails to match";
const OWN_FOLLOW_ROW: &str =
    "deletes only the caller's own follow row, and `member_id` is held to the caller";
const SEARCH_FILTER: &str =
    "narrows results the search already filters to what the caller may read";
const WORKSPACE_MEMBER_IN_STORE: &str =
    "the store refuses a member outside the guarded thread's workspace";
const GRANT_MEMBER: &str =
    "the handler requires a member of the caller's workspace (`get_member_in`)";

/// Every id argument of every tool, and what binds it. The checking rows come
/// first, in the order the checks run: channel, thread, message, references.
const ROWS: &[Row] = &[
    // Checked here.
    row("channel_id", IdCheck::Channel, CHANNEL_SCOPED_TOOLS),
    row(
        "channel_id",
        IdCheck::Channel,
        CHANNEL_AND_THREAD_SCOPED_TOOLS,
    ),
    row("thread_id", IdCheck::Thread, THREAD_SCOPED_TOOLS),
    row(
        "thread_id",
        IdCheck::Thread,
        CHANNEL_AND_THREAD_SCOPED_TOOLS,
    ),
    row("message_id", IdCheck::Message, MESSAGE_SCOPED_TOOLS),
    row(
        "src_id",
        IdCheck::ReferenceEnd {
            kind_key: "src_kind",
        },
        REFERENCE_TOOLS,
    ),
    row(
        "dst_id",
        IdCheck::ReferenceEnd {
            kind_key: "dst_kind",
        },
        REFERENCE_TOOLS,
    ),
    row("member_id", IdCheck::Caller, super::MEMBER_SCOPED_TOOLS),
    // A channel named but not checked here.
    row(
        "channel_id",
        IdCheck::Exempt(
            "an admin manages a private channel it is not in, so the handler holds the channel \
             to the caller's workspace instead (`channel::own_channel`)",
        ),
        &[
            "add_channel_member",
            "list_channel_members",
            "remove_channel_member",
        ],
    ),
    row(
        "channel_id",
        IdCheck::Exempt(
            "the store requires the channel, its owner and its creator to be live in the \
             ticket's workspace",
        ),
        &["create_share_ticket"],
    ),
    row(
        "channel_id",
        IdCheck::Exempt(OWN_FOLLOW_ROW),
        &["unfollow_channel"],
    ),
    row(
        "channel_id",
        IdCheck::Exempt("the guard checks the `message_id` the seed is taken from"),
        &["seed_from_message"],
    ),
    row(
        "channel_id",
        IdCheck::Exempt(SEARCH_FILTER),
        &["search_messages"],
    ),
    // A thread named but not checked here.
    row(
        "thread_id",
        IdCheck::Exempt(OWN_FOLLOW_ROW),
        &["unfollow_thread"],
    ),
    row(
        "thread_id",
        IdCheck::Exempt(
            "the handler authorizes the thread (`authorize_thread`) and links it in that \
             thread's workspace",
        ),
        &["link_slack_channel", "link_github_issue"],
    ),
    row(
        "depends_on_thread_id",
        IdCheck::Exempt(
            "the handler checks access to it and that it shares the guarded thread's workspace",
        ),
        &["add_thread_dependency"],
    ),
    row(
        "depends_on_thread_id",
        IdCheck::Exempt("removes only an edge from the guarded thread"),
        &["remove_thread_dependency"],
    ),
    row(
        "parent_thread_id",
        IdCheck::Exempt("the store requires the parent to be in the guarded channel"),
        &["create_thread"],
    ),
    row(
        "supersedes",
        IdCheck::Exempt(
            "the handler authorizes it (`authorize_thread`) and requires it in the channel's \
             workspace",
        ),
        &["create_thread"],
    ),
    row(
        "duplicate_of",
        IdCheck::Exempt(
            "the store requires a live thread the caller can read in the same workspace",
        ),
        &["close_abandoned_thread"],
    ),
    // A message named but not checked here.
    row(
        "message_id",
        IdCheck::Exempt(
            "the guard checks `thread_id`, and the store refuses a message outside that thread",
        ),
        &["pin_message", "unpin_message"],
    ),
    // A member named, other than the caller.
    row(
        "member_id",
        IdCheck::Exempt(
            "work state, cross-member by design; the handlers enforce workspace and channel \
             access",
        ),
        super::MEMBER_WORK_STATE_TOOLS,
    ),
    row(
        "member_id",
        IdCheck::Exempt(
            "the target of an administrative or routing act, not the caller; a legitimate \
             target by design",
        ),
        super::MEMBER_TARGET_TOOLS,
    ),
    row(
        "member_id",
        IdCheck::Exempt(
            "personal state or an administrative target depending on another argument, so the \
             handler decides",
        ),
        super::MEMBER_ARGUMENT_SCOPED_TOOLS,
    ),
    row(
        "author_id",
        IdCheck::Exempt(SEARCH_FILTER),
        &["search_messages"],
    ),
    row(
        "assignee_id",
        IdCheck::Exempt(WORKSPACE_MEMBER_IN_STORE),
        &["assign_thread"],
    ),
    row(
        "owner_id",
        IdCheck::Exempt(WORKSPACE_MEMBER_IN_STORE),
        &["set_thread_owner"],
    ),
    row(
        "subject_id",
        IdCheck::Exempt(GRANT_MEMBER),
        &["create_delegation_grant"],
    ),
    row(
        "delegate_id",
        IdCheck::Exempt(GRANT_MEMBER),
        &["create_delegation_grant"],
    ),
    row(
        "followed_member_id",
        IdCheck::Exempt(
            "the handler requires a member of the follower's workspace (`get_member_in`)",
        ),
        &["follow_member"],
    ),
    row(
        "followed_member_id",
        IdCheck::Exempt(OWN_FOLLOW_ROW),
        &["unfollow_member"],
    ),
    row(
        "other_member_id",
        IdCheck::Exempt(
            "the store refuses a member outside the named workspace, which is the caller's",
        ),
        &["open_dm_conversation"],
    ),
    row(
        "member_ids",
        IdCheck::Exempt(
            "the store requires every member in the named workspace, which is the caller's",
        ),
        &["open_group_dm"],
    ),
    // Positions and tokens, which name nothing to read.
    row(
        "since_log_id",
        IdCheck::Exempt(LOG_POSITION),
        &[
            "wait_for_result",
            "wait_for_ready",
            "wait_for_claim_expired",
            "wait_for_claim_failed",
            "wait_for_blocked_resolved",
            "wait_for_landed",
            "wait_for_mention",
            "wait_for_notification",
        ],
    ),
    row(
        "after_log_id",
        IdCheck::Exempt(LOG_POSITION),
        &["wait_for_event"],
    ),
    row(
        "cursor",
        IdCheck::Exempt(PAGE_POSITION),
        &["list_threads", "find_threads"],
    ),
    row(
        "message_cursor",
        IdCheck::Exempt(PAGE_POSITION),
        &["get_thread_context"],
    ),
    row(
        "claim_lease_id",
        IdCheck::Exempt(FENCING_TOKEN),
        &[
            "report_usage",
            "renew_claim",
            "acknowledge_claim",
            "release_claim",
            "set_thread_result",
        ],
    ),
    row(
        "lease_id",
        IdCheck::Exempt(FENCING_TOKEN),
        &["submit_plan", "amend_plan"],
    ),
    row(
        "usage_report_id",
        IdCheck::Exempt("an idempotency key the caller chooses, not an entity"),
        &["report_usage"],
    ),
    row(
        "parent_run_id",
        IdCheck::Exempt(
            "a producer's opaque run label: the reads filter it to the caller's workspace and \
             the write lands on the guarded thread",
        ),
        &[
            "set_thread_lineage",
            "list_run_threads",
            "get_run_occupancy",
        ],
    ),
    row(
        "slack_channel_id",
        IdCheck::Exempt(
            "an id in Slack, not in Maidan: a link binds it to an authorized thread, and an \
             unlink removes only a link in the caller's workspace",
        ),
        &["link_slack_channel", "unlink_slack_channel"],
    ),
    // Other entities, each looked up inside the caller's scope by its handler.
    row(
        "block_id",
        IdCheck::Exempt("detaches only from the guarded thread"),
        &["detach_memory_block"],
    ),
    row(
        "block_id",
        IdCheck::Exempt("the handler finds the block only in the caller's workspace"),
        &["delete_memory_block"],
    ),
    row(
        "recipe_id",
        IdCheck::Exempt("the handler requires the recipe in the guarded channel's workspace"),
        &["create_task_schedule"],
    ),
    row(
        "recipe_id",
        IdCheck::Exempt(
            "the handler finds the recipe only in the caller's workspace, then checks access \
             to its channel",
        ),
        &["instantiate_recipe", "delete_recipe"],
    ),
    row(
        "schedule_id",
        IdCheck::Exempt(
            "the handler holds the schedule to the caller's workspace and its channel \
             (`authorize_schedule`)",
        ),
        &["set_task_schedule_active", "delete_task_schedule"],
    ),
    row(
        "gate_id",
        IdCheck::Exempt("the handler finds the gate only in the caller's workspace"),
        &["get_approval_gate", "approval_decide"],
    ),
    row(
        "grant_id",
        IdCheck::Exempt("the handler finds the grant only in the caller's workspace"),
        &["delegate_token", "revoke_delegation_grant"],
    ),
    row(
        "hook_id",
        IdCheck::Exempt("the store revokes it only in the named workspace, which is the caller's"),
        &["revoke_fsm_hook"],
    ),
    row(
        "command_id",
        IdCheck::Exempt("the store revokes it only in the named workspace, which is the caller's"),
        &["revoke_slash_command"],
    ),
    row(
        "delivery_id",
        IdCheck::Exempt("looked up under the guarded thread"),
        &["replay_result_delivery"],
    ),
    row(
        "notification_id",
        IdCheck::Exempt("acts only on a notification of `member_id`, which is held to the caller"),
        &["mark_notification_read", "snooze_notification"],
    ),
    row(
        "ticket_id",
        IdCheck::Exempt("the store revokes a ticket only in the caller's workspace"),
        &["revoke_share_ticket"],
    ),
    row(
        "upload_id",
        IdCheck::Exempt(
            "the handler holds the upload to the caller's workspace (`validate_for_workspace`)",
        ),
        &[
            "upload_artifact_multipart_part",
            "complete_artifact_multipart",
            "abort_artifact_multipart",
        ],
    ),
    row(
        "dm_conversation_id",
        IdCheck::Exempt("the handler requires the caller to be a participant"),
        &["post_dm_message"],
    ),
    row(
        "group_dm_conversation_id",
        IdCheck::Exempt("the handler requires the caller to be a participant"),
        &["get_group_dm", "post_group_dm_message"],
    ),
    row(
        "sha256",
        IdCheck::Exempt("the store links only an artifact the guarded thread's workspace holds"),
        &["link_thread_artifact"],
    ),
    row(
        "sha256",
        IdCheck::Exempt("removes only a link of the guarded thread"),
        &["unlink_thread_artifact"],
    ),
    row(
        "sha256",
        IdCheck::Exempt("a digest recorded on the guarded thread; nothing is read by it"),
        &["add_evidence_record"],
    ),
    row(
        "sha256",
        IdCheck::Exempt("the handler reads only an artifact the caller's workspace references"),
        &["get_artifact", "get_artifact_metadata"],
    ),
];

/// Whether a tool argument names something by id: its name says so (`*_id`,
/// `*_ids`, an artifact's `sha256`), or its schema declares a UUID, which
/// catches the ids named otherwise (`supersedes`, `duplicate_of`, cursors).
pub(super) fn names_an_id(argument: &str, schema: &Value) -> bool {
    argument.ends_with("_id")
        || argument.ends_with("_ids")
        || argument == "sha256"
        || declares_uuid(schema)
}

fn declares_uuid(schema: &Value) -> bool {
    match schema {
        Value::Object(map) => {
            map.get("format").and_then(Value::as_str) == Some("uuid")
                || map.values().any(declares_uuid)
        }
        Value::Array(items) => items.iter().any(declares_uuid),
        _ => false,
    }
}

fn id_arguments_of(input_schema: &Value) -> Vec<String> {
    let mut ids: Vec<String> = input_schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .filter(|(name, schema)| names_an_id(name, schema))
                .map(|(name, _)| name.clone())
                .collect()
        })
        .unwrap_or_default();
    ids.sort_unstable();
    ids
}

/// The id arguments a tool's schema declares, or `None` for no such tool.
fn id_arguments(tool: &str) -> Option<&'static [String]> {
    #[cfg(test)]
    {
        if let Some(planted) = planted::id_arguments(tool) {
            return Some(planted);
        }
    }
    static IDS: LazyLock<HashMap<String, Vec<String>>> = LazyLock::new(|| {
        super::catalog()
            .into_iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?.to_owned();
                let schema = tool.get("inputSchema").cloned().unwrap_or(Value::Null);
                Some((name, id_arguments_of(&schema)))
            })
            .collect()
    });
    IDS.get(tool).map(Vec::as_slice)
}

/// What binds `argument` when `tool` is called, or `None` if nobody decided.
pub(super) fn classify(tool: &str, argument: &str) -> Option<IdCheck> {
    if argument == "workspace_id" {
        return Some(IdCheck::Workspace);
    }
    ROWS.iter()
        .find(|row| row.argument == argument && row.tools.contains(&tool))
        .map(|row| row.check)
}

/// Refuses a tool with an id argument that has no row. A tool with no schema
/// gets the answer an unknown name gets.
fn refuse_unclassified(tool: &str) -> Result<(), McpError> {
    let Some(arguments) = id_arguments(tool) else {
        return Err(McpError::MethodNotFound(format!("tools/{tool}")));
    };
    let unclassified: Vec<&str> = arguments
        .iter()
        .map(String::as_str)
        .filter(|argument| classify(tool, argument).is_none())
        .collect();
    if unclassified.is_empty() {
        return Ok(());
    }
    tracing::error!(
        tool,
        ?unclassified,
        "an MCP tool takes ids no access check is classified for; refused"
    );
    Err(McpError::Forbidden(format!(
        "{tool} takes {} with no access check classified for it",
        unclassified.join(", ")
    )))
}

/// The pre-dispatch id guard. Refuses an unclassified tool for every caller;
/// for a caller with auth on, runs each checking row the tool is in. An id
/// that is absent or malformed is left to the handler's own decode error.
pub(super) async fn enforce(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    name: &str,
    args: &Value,
) -> Result<(), McpError> {
    refuse_unclassified(name)?;
    let store = server.store.as_ref();
    let field = |key: &str| -> Option<uuid::Uuid> {
        args.get(key)
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
    };
    for row in ROWS.iter().filter(|row| row.tools.contains(&name)) {
        let Some(id) = field(row.argument) else {
            continue;
        };
        match row.check {
            IdCheck::Channel => {
                maidan_auth::ensure_channel_access(store, auth, ChannelId(id)).await?;
            }
            IdCheck::Thread => {
                maidan_auth::ensure_thread_access(store, auth, ThreadId(id)).await?;
            }
            IdCheck::Message => {
                maidan_auth::ensure_message_access(store, auth, MessageId(id)).await?;
            }
            IdCheck::ReferenceEnd { kind_key } => {
                let kind = args
                    .get(kind_key)
                    .and_then(|kind| serde_json::from_value::<RefSide>(kind.clone()).ok());
                match kind {
                    Some(RefSide::Thread) => {
                        maidan_auth::ensure_thread_access(store, auth, ThreadId(id)).await?;
                    }
                    Some(RefSide::Message) => {
                        maidan_auth::ensure_message_access(store, auth, MessageId(id)).await?;
                    }
                    None => {}
                }
            }
            IdCheck::Exempt(reason) => {
                tracing::trace!(
                    tool = name,
                    argument = row.argument,
                    reason,
                    "id left to its handler"
                );
            }
            IdCheck::Workspace | IdCheck::Caller => {}
        }
    }
    Ok(())
}

/// Tools registered by a test alone, to show what the guard does with a tool
/// nobody classified.
#[cfg(test)]
pub(super) mod planted {
    use std::sync::Mutex;

    use serde_json::Value;

    static PLANTED: Mutex<Vec<(String, &'static [String])>> = Mutex::new(Vec::new());

    /// Registers `tool` with `input_schema` for the guard to read.
    pub(in crate::tools) fn plant(tool: &str, input_schema: &Value) {
        let ids: &'static [String] = Vec::leak(super::id_arguments_of(input_schema));
        PLANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((tool.to_owned(), ids));
    }

    pub(super) fn id_arguments(tool: &str) -> Option<&'static [String]> {
        PLANTED
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|(name, _)| name == tool)
            .map(|(_, ids)| *ids)
    }
}

#[cfg(test)]
mod classification_tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::tools::{catalog, declared_arguments};

    fn tools_declaring(argument: &str) -> Vec<String> {
        catalog()
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?;
                declared_arguments(name)?
                    .contains(argument)
                    .then(|| name.to_owned())
            })
            .collect()
    }

    /// Every tool that declares `argument` is in exactly one row for it, and
    /// every row for it names only tools that declare it.
    fn assert_classified(argument: &str, at_least: usize) -> Vec<String> {
        let declared = tools_declaring(argument);
        assert!(
            declared.len() >= at_least,
            "expected at least {at_least} tools with {argument}; found {} — the catalog parse \
             broke, not the guard",
            declared.len()
        );
        for name in &declared {
            let claimed = ROWS
                .iter()
                .filter(|row| row.argument == argument && row.tools.contains(&name.as_str()))
                .count();
            assert_eq!(
                claimed, 1,
                "{argument} of {name} must be in exactly one row, found {claimed}"
            );
        }
        for row in ROWS.iter().filter(|row| row.argument == argument) {
            for name in row.tools {
                assert!(
                    declared.iter().any(|d| d == name),
                    "{name} is classified for {argument} but declares no {argument}"
                );
            }
        }
        declared
    }

    /// A tool that names a channel is checked for access to it before
    /// dispatch or listed with the reason it is not. The channel-membership
    /// tools were neither, and let one workspace's admin list and empty
    /// another's private channel.
    #[test]
    fn every_channel_id_tool_is_classified() {
        assert_classified("channel_id", 16);
    }

    /// A tool that names a thread is checked for access to it before dispatch
    /// or listed with the reason it is not.
    #[test]
    fn every_thread_id_tool_is_classified() {
        let declared = assert_classified("thread_id", 80);
        let checked = declared
            .iter()
            .filter(|name| classify(name, "thread_id") == Some(IdCheck::Thread))
            .count();
        assert!(
            checked + 5 >= declared.len(),
            "only {checked} of {} thread_id tools are checked before dispatch; an exemption is \
             for a tool whose handler binds the thread, not the default",
            declared.len()
        );
    }

    /// A tool that names a message is checked for access to it before
    /// dispatch or listed with the reason it is not.
    #[test]
    fn every_message_id_tool_is_classified() {
        assert_classified("message_id", 12);
    }

    /// The guard's contract: every id argument of every tool in the catalog
    /// has exactly one decision, so the refusal in [`enforce`] never fires in
    /// a build whose tests pass.
    #[test]
    fn every_id_argument_of_every_tool_is_classified() {
        let mut pairs = 0;
        let mut missing = Vec::new();
        let mut doubled = Vec::new();
        for tool in catalog() {
            let name = tool["name"].as_str().unwrap_or_default().to_owned();
            for argument in id_arguments_of(&tool["inputSchema"]) {
                pairs += 1;
                let claimed = ROWS
                    .iter()
                    .filter(|row| row.argument == argument && row.tools.contains(&name.as_str()))
                    .count();
                if argument == "workspace_id" {
                    assert_eq!(claimed, 0, "{name}: workspace_id needs no row");
                } else if claimed == 0 {
                    missing.push(format!("{name}.{argument}"));
                } else if claimed > 1 {
                    doubled.push(format!("{name}.{argument}"));
                }
            }
        }
        assert!(
            pairs > 250,
            "expected broad id coverage; found {pairs} — the schema walk broke, not the guard"
        );
        assert!(
            missing.is_empty(),
            "id arguments no row classifies: {missing:?}. Check each in the guard \
             (Channel, Thread, Message) or exempt it with the reason its handler binds it."
        );
        assert!(
            doubled.is_empty(),
            "id arguments in more than one row: {doubled:?}"
        );
    }

    /// A row may not name a tool that does not exist or an argument the tool
    /// does not take: a renamed argument would leave its row checking nothing.
    #[test]
    fn every_row_names_a_real_tool_and_an_id_it_takes() {
        let names: BTreeSet<String> = catalog()
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect();
        for row in ROWS {
            for tool in row.tools {
                assert!(names.contains(*tool), "{tool} is not a tool");
                let ids = id_arguments(tool).unwrap_or_default();
                assert!(
                    ids.iter().any(|id| id == row.argument),
                    "{tool} is classified for {} but declares no such id",
                    row.argument
                );
            }
            if let IdCheck::Exempt(reason) = row.check {
                assert!(
                    !reason.trim().is_empty(),
                    "{}: an exemption needs a reason",
                    row.argument
                );
            }
        }
    }

    #[test]
    fn an_id_is_found_by_its_name_or_its_uuid_format() {
        assert!(names_an_id("thread_id", &json!({ "type": "string" })));
        assert!(names_an_id("member_ids", &json!({ "type": "array" })));
        assert!(names_an_id("sha256", &json!({ "type": "string" })));
        assert!(names_an_id(
            "supersedes",
            &json!({ "type": "string", "format": "uuid" })
        ));
        assert!(names_an_id(
            "targets",
            &json!({ "type": "array", "items": { "type": "string", "format": "uuid" } })
        ));
        assert!(!names_an_id("title", &json!({ "type": "string" })));
    }
}

/// The guard in a running server: an unclassified tool is refused, and a
/// classified one still refuses another workspace's ids.
#[cfg(test)]
mod dispatch_tests {
    use std::sync::Arc;

    use maidan_artifacts::LocalFsStore;
    use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
    use maidan_auth::AuthContext;
    use maidan_search::HashV1Provider;
    use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
    use maidan_types::*;
    use serde_json::json;

    use super::{classify, planted, IdCheck};
    use crate::call_context::CallContext;
    use crate::error::McpError;
    use crate::server::McpServer;

    struct Tenant {
        member: MemberId,
        workspace: WorkspaceId,
        thread: ThreadId,
    }

    async fn tenant(store: &Arc<dyn Store>, name: &str) -> Tenant {
        let workspace = store
            .create_workspace(NewWorkspace { name: name.into() })
            .await
            .unwrap();
        let member = store
            .create_member(NewMember {
                workspace_id: workspace.id,
                handle: format!("{name}-agent"),
                display_name: None,
                kind: MemberKind::Agent,
            })
            .await
            .unwrap();
        let channel = store
            .create_channel(NewChannel {
                workspace_id: workspace.id,
                name: "general".into(),
                topic: None,
                private: false,
            })
            .await
            .unwrap();
        let thread = store
            .create_thread(NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some(format!("{name} work")),
                description: None,
                created_by: None,
            })
            .await
            .unwrap();
        Tenant {
            member: member.id,
            workspace: workspace.id,
            thread: thread.id,
        }
    }

    async fn two_tenants() -> (McpServer, Tenant, Tenant) {
        let pool = maidan_search::sqlite_pool_options()
            .max_connections(2)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .unwrap();
        run_sqlite_migrations(&pool).await.unwrap();
        let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
        let victim = tenant(&store, "victim").await;
        let attacker = tenant(&store, "attacker").await;
        let server = McpServer::new(
            store,
            Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
            Arc::new(maidan_search::SqliteSearch::new(pool)),
            Arc::new(HashV1Provider),
        );
        (server, victim, attacker)
    }

    fn token(tenant: &Tenant) -> AuthContext {
        AuthContext::from_token(
            ApiTokenId::new(),
            tenant.member,
            tenant.workspace,
            vec![WORKSPACE_READ.to_string(), THREAD_TRANSITION.to_string()],
        )
    }

    /// The defect MCP-005 named: a tool that takes a `thread_id`, registered
    /// and reachable but in no list. With the old `_ => {}` default it passed
    /// the guard and reached dispatch (here, `MethodNotFound`, since the
    /// planted tool has no handler); now the guard refuses it first, for the
    /// thread's own workspace as much as for another.
    #[tokio::test]
    async fn an_unclassified_tool_with_a_thread_id_is_refused_before_it_runs() {
        let (server, victim, attacker) = two_tenants().await;
        let tool = "planted_unclassified_thread_tool";
        planted::plant(
            tool,
            &json!({
                "type": "object",
                "properties": { "thread_id": { "type": "string", "format": "uuid" } },
            }),
        );
        let args = json!({ "thread_id": victim.thread.0 });
        let call = CallContext::from_params(&json!({}));
        for auth in [token(&attacker), token(&victim)] {
            let err = crate::tools::dispatch(&server, &auth, tool, &args, &call)
                .await
                .expect_err("an unclassified tool must not run");
            match err {
                McpError::Forbidden(message) => assert!(
                    message.contains("thread_id"),
                    "the refusal names the unclassified argument: {message}"
                ),
                other => panic!("expected the guard's refusal, got {other}"),
            }
        }
    }

    /// The classification kept its teeth: a thread-scoped tool whose handler
    /// has no check of its own still refuses another workspace's thread, for
    /// a read and for a write, while the owner's own call succeeds.
    #[tokio::test]
    async fn a_thread_scoped_tool_refuses_another_workspaces_thread() {
        let (server, victim, attacker) = two_tenants().await;
        assert_eq!(
            classify("get_thread_status", "thread_id"),
            Some(IdCheck::Thread)
        );
        assert_eq!(
            classify("rename_thread", "thread_id"),
            Some(IdCheck::Thread)
        );

        let foreign = json!({ "thread_id": victim.thread.0 });
        let err = server
            .call_tool(&token(&attacker), "get_thread_status", &foreign)
            .await
            .expect_err("another workspace's thread status must be refused");
        assert!(
            matches!(err, McpError::Forbidden(_) | McpError::NotFound),
            "expected a refusal, got {err}"
        );
        let rename = json!({ "thread_id": victim.thread.0, "title": "taken" });
        let err = server
            .call_tool(&token(&attacker), "rename_thread", &rename)
            .await
            .expect_err("another workspace's thread must not be renamed");
        assert!(
            matches!(err, McpError::Forbidden(_) | McpError::NotFound),
            "expected a refusal, got {err}"
        );
        let stored = server.store.get_thread(victim.thread).await.unwrap();
        assert_eq!(stored.title.as_deref(), Some("victim work"));

        server
            .call_tool(&token(&victim), "get_thread_status", &foreign)
            .await
            .expect("the owner reads its own thread's status");
    }
}
