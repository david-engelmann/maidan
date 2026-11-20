//! The root `plugin.json`, `skills/` and `.github/plugin/marketplace.json`
//! are the Copilot CLI package (Open Work Next 10), in the Agent Plugins 1.0
//! shape. Agent Plugins 1.0 passes remote MCP config literally, so the plugin
//! carries no MCP server (and no host or token): Copilot CLI adds the server
//! with `copilot mcp add`. The bundled skills stay copies of the examples.

use std::path::{Path, PathBuf};

use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(path: &str) -> Value {
    let text = std::fs::read_to_string(repo().join(path)).expect("manifest exists");
    serde_json::from_str(&text).expect("manifest is JSON")
}

#[test]
fn the_copilot_plugin_is_an_agent_plugins_manifest_with_no_server_or_secret() {
    let m = json("plugin.json");
    assert_eq!(
        m["$schema"],
        "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
    );
    assert_eq!(m["name"], "maidan");
    let desc = m["description"].as_str().unwrap_or_default();
    for word in ["Free", "self-hosted", "copilot mcp add", "Maidan console"] {
        assert!(desc.contains(word), "description lacks {word:?}");
    }
    for field in ["mcpServers", "skills", "hooks", "agents"] {
        assert!(
            m.get(field).is_none(),
            "{field} is not an Agent Plugins 1.0 field"
        );
    }
    assert!(
        !repo().join("mcp.json").exists(),
        "no literal remote server config"
    );
    let text = serde_json::to_string(&m).expect("serialize");
    assert!(!text.contains("Bearer") && !text.contains("maidan.world"));
}

#[test]
fn the_bundled_skills_match_the_examples() {
    for skill in ["maidan-claimant", "maidan-reviewer"] {
        let ours = repo().join("skills").join(skill);
        let theirs = repo().join("examples/claude-code").join(skill);
        let mut names: Vec<_> = std::fs::read_dir(&theirs)
            .expect("example skill")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .collect();
        names.sort();
        assert!(names.iter().any(|n| n == "SKILL.md"));
        for name in names {
            let read = |dir: &Path| std::fs::read(dir.join(&name)).expect("skill file");
            assert_eq!(read(&ours), read(&theirs), "{skill}/{name:?} drifted");
        }
    }
}

#[test]
fn the_marketplace_lists_the_root_plugin() {
    let m = json(".github/plugin/marketplace.json");
    assert!(m["owner"]["name"].is_string());
    let plugins = m["plugins"].as_array().expect("plugins");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0]["name"], "maidan");
    assert_eq!(plugins[0]["source"], ".");
}
