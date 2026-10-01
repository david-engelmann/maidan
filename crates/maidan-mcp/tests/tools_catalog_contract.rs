//! Golden file for MCP tool names.

use std::path::PathBuf;

#[test]
fn mcp_tool_names_match_contract_file() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../../contracts/mcp-tool-names.json");
    let expected: Vec<String> = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("contract json");
    let mut actual: Vec<String> = maidan_mcp::tools::catalog()
        .into_iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .collect();
    actual.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(
        actual, expected,
        "update contracts/mcp-tool-names.json if intentional"
    );
}

#[test]
fn acting_identity_is_absent_from_public_tool_schemas() {
    let catalog = maidan_mcp::tools::catalog();
    let forbidden = [
        ("open_dm_conversation", "member_id"),
        ("post_dm_message", "author_id"),
        ("assign_thread", "actor_id"),
        ("claim_thread", "member_id"),
        ("unassign_thread", "actor_id"),
        ("transition_thread", "actor_id"),
        ("claim_next_thread", "member_id"),
        ("renew_claim", "member_id"),
        ("acknowledge_claim", "member_id"),
        ("release_claim", "member_id"),
        ("post_message", "author_id"),
        ("edit_message", "editor_id"),
        ("cast_vote", "member_id"),
        ("add_reaction", "member_id"),
        ("remove_reaction", "member_id"),
        ("pin_message", "member_id"),
        ("unpin_message", "member_id"),
        ("upload_artifact", "uploaded_by"),
        ("complete_artifact_multipart", "uploaded_by"),
        ("link_slack_channel", "member_id"),
        ("link_github_issue", "member_id"),
    ];

    for (tool_name, field) in forbidden {
        let tool = catalog
            .iter()
            .find(|tool| tool["name"] == tool_name)
            .unwrap_or_else(|| panic!("missing tool {tool_name}"));
        let properties = tool["inputSchema"]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{tool_name} has no object input schema"));
        assert!(
            !properties.contains_key(field),
            "{tool_name} still lets the caller select acting identity through {field}"
        );
    }
}

/// Pairs whose names invite a mix-up: each description names the other, so
/// an agent that picked the wrong one learns which it wanted.
#[test]
fn easily_confused_tools_name_each_other() {
    let catalog = maidan_mcp::tools::catalog();
    let description = |name: &str| -> String {
        catalog
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing tool {name}"))["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{name} has no description"))
            .to_string()
    };
    for (tool, names) in [
        ("set_wait", "wait_for_"),
        ("get_inbox", "get_waiting_inbox"),
        ("get_waiting_inbox", "get_inbox"),
        ("wait_for_result", "set_wait"),
        ("wait_for_ready", "set_wait"),
    ] {
        assert!(
            description(tool).contains(names),
            "{tool}'s description does not point at {names}"
        );
    }
}
