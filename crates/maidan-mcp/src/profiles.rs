//! Tool profiles: a short, fixed `tools/list` on its own endpoint
//! (`POST /mcp/worker`, `POST /mcp/reviewer`).
//!
//! The catalog on `POST /mcp` is filtered by the token, so two agents with
//! different tokens get different lists and cannot share a cached prompt
//! prefix. A profile's list is the same bytes for every caller, sorted by
//! name, so it can be cached `public`. What a token may do is decided when it
//! calls: a profile tool the token cannot call is refused then, and a tool
//! that is not in the profile is refused on that endpoint.

use serde_json::Value;

use crate::tools;

/// A fixed tool list with its own endpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Claim, read, work, report, hand back, release.
    Worker,
    /// Read what is waiting, decide, and close an approved thread.
    Reviewer,
}

/// The worker profile, in name order. The waiter loop and nothing else:
/// claiming, the lease and its working clock, the task's context, usage, the
/// result and its hand-off, a message, a human approval, and the wait that
/// replaces polling an empty queue.
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

/// The reviewer profile, in name order: what is waiting, the work under
/// review (context, messages, result, artifacts), the review standing, the
/// verdict, a reply, and the transition that closes an approved thread.
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

    /// The profile's `tools/list` entries, sorted by name. Catalog entries are
    /// taken unchanged, so a tool reads the same on every endpoint.
    pub fn catalog(self) -> Vec<Value> {
        let mut listed: Vec<Value> = tools::catalog()
            .into_iter()
            .filter(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| self.contains(name))
            })
            .collect();
        listed.sort_by(|left, right| tool_name(left).cmp(tool_name(right)));
        listed
    }
}

fn tool_name(tool: &Value) -> &str {
    tool.get("name").and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_profile_names_only_catalog_tools_in_order_without_repeats() {
        let catalog = tools::catalog();
        let catalog_names: Vec<&str> = catalog
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        for profile in Profile::ALL {
            let names = profile.tool_names();
            let mut sorted = names.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(names, sorted.as_slice(), "{} is not sorted", profile.name());
            assert!(
                names.iter().all(|name| catalog_names.contains(name)),
                "{} names a tool the catalog does not have",
                profile.name()
            );
            let listed_catalog = profile.catalog();
            let listed: Vec<&str> = listed_catalog
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect();
            assert_eq!(listed, names, "{}", profile.name());
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

    #[test]
    fn each_profile_matches_its_byte_golden() {
        for profile in Profile::ALL {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(format!("profile-{}.json", profile.name()));
            let bytes = serde_json::to_vec(&profile.catalog()).expect("profile json");
            if std::env::var_os("UPDATE_PROFILE_GOLDEN").is_some() {
                std::fs::write(&path, &bytes).expect("write golden");
            }
            let golden = std::fs::read(&path).unwrap_or_default();
            assert_eq!(
                bytes,
                golden,
                "{} drifted from {}; regenerate the golden",
                profile.name(),
                path.display()
            );
        }
    }
}
