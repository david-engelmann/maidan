//! Tool profiles: a short, fixed `tools/list` served on an endpoint of its own
//! (`POST /mcp/worker`, `POST /mcp/reviewer`), for a harness that loads every
//! tool definition into each request.
//!
//! The full catalog on `/mcp` is filtered by the token's capabilities, so two
//! agents with different tokens get different lists and cannot share a cached
//! prompt prefix. A profile's list is the same bytes for every caller, sorted
//! by name, so it can be cached `public` and shared across a fleet. What a
//! token may do is decided when it calls: a profile tool the token lacks the
//! capability for is refused then, as it would be on `/mcp`, and a tool outside
//! the profile is refused on the profile's endpoint.

use serde_json::Value;

use crate::tools;

/// A fixed tool list with its own endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// The waiter loop: claim, read, work, report, hand back, release.
    Worker,
    /// Find what waits on you, read it, and decide.
    Reviewer,
}

/// The worker profile, in name order. Each step of the waiter loop
/// (Integration, "The waiter loop") and nothing else: claiming, the lease and
/// its working clock, the task's context, usage, the result and its hand-off
/// to review, a message on the thread, a human approval, and the wait for
/// ready work that replaces polling an empty queue.
pub const WORKER_TOOLS: &[&str] = &[
    "acknowledge_claim",
    "claim_next_thread",
    "claim_next_workspace_thread",
    "get_approval_gate",
    "get_thread_context",
    "post_message",
    "release_claim",
    "renew_claim",
    "report_usage",
    "request_approval",
    "set_thread_result",
    "transition_thread",
    "wait_for_ready",
    "whoami",
];

/// The reviewer profile, in name order: the reviews requested from you, the
/// work under review (its context, messages, result and artifacts), the
/// review standing and verdicts, your own verdict, a reply on the thread, and
/// the transition that closes an approved thread.
pub const REVIEWER_TOOLS: &[&str] = &[
    "get_artifact",
    "get_review_status",
    "get_thread_context",
    "get_thread_result",
    "get_waiting_inbox",
    "list_messages",
    "list_reviews",
    "post_message",
    "submit_review",
    "transition_thread",
    "wait_for_notification",
    "whoami",
];

impl Profile {
    pub const ALL: [Profile; 2] = [Profile::Worker, Profile::Reviewer];

    pub fn name(self) -> &'static str {
        match self {
            Profile::Worker => "worker",
            Profile::Reviewer => "reviewer",
        }
    }

    /// The HTTP endpoint that serves this profile.
    pub fn path(self) -> &'static str {
        match self {
            Profile::Worker => "/mcp/worker",
            Profile::Reviewer => "/mcp/reviewer",
        }
    }

    pub fn tool_names(self) -> &'static [&'static str] {
        match self {
            Profile::Worker => WORKER_TOOLS,
            Profile::Reviewer => REVIEWER_TOOLS,
        }
    }

    pub fn contains(self, tool: &str) -> bool {
        self.tool_names().contains(&tool)
    }

    /// The profile's `tools/list` entries, sorted by name. The catalog entries
    /// are taken unchanged, so a tool reads the same on every endpoint.
    pub fn catalog(self) -> Vec<Value> {
        let mut listed: Vec<Value> = tools::catalog()
            .into_iter()
            .filter(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| self.contains(name))
            })
            .collect();
        listed.sort_by(|a, b| {
            let name = |tool: &Value| tool.get("name").and_then(Value::as_str).map(str::to_owned);
            name(a).cmp(&name(b))
        });
        listed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_profile_names_only_catalog_tools_in_order_without_repeats() {
        for profile in Profile::ALL {
            let names = profile.tool_names();
            let mut sorted = names.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(names, sorted.as_slice(), "{} is not sorted", profile.name());
            let listed: Vec<String> = profile
                .catalog()
                .iter()
                .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                .collect();
            assert_eq!(
                listed,
                names.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
                "{} names a tool the catalog does not have",
                profile.name()
            );
        }
    }

    #[test]
    fn every_profile_tool_has_a_capability() {
        for profile in Profile::ALL {
            for name in profile.tool_names() {
                assert!(
                    tools::required_capability(name).is_ok(),
                    "{name} in the {} profile has no capability",
                    profile.name()
                );
            }
        }
    }
}
