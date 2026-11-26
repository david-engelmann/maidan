//! Role playbooks, served over MCP.
//!
//! The Claude Code skills under `examples/claude-code/maidan-<role>/SKILL.md`
//! say how the tools fit together for one job: a claimant's loop, a reviewer's
//! verdict, a dispatcher's filing. A client that is not Claude Code never sees
//! those files, so this module compiles the same files in and serves each as a
//! prompt (`playbook_<role>`) and a resource (`maidan://playbooks/<role>`).
//! One edit to a skill updates both audiences, and nothing is copied.
//!
//! A playbook holds no caller's data, so it is the same for everyone and
//! needs no more than the capability `prompts/get` and `resources/read`
//! already ask for. It is served on every endpoint, the profile endpoints
//! included: a profile narrows `tools/list` and `tools/call`, not what a
//! client may read about using them.
//!
//! The server image copies `examples/` into its build context (`.dockerignore`
//! leaves it in; it drops `docs/`), which `include_str!` needs. A skill added
//! under `examples/claude-code/` must also be added to [`SOURCES`]:
//! `every_skill_on_disk_is_served_and_nothing_else` fails until it is.

use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::error::McpError;

/// Prefix of a playbook's prompt name: `playbook_<role>`.
pub const PROMPT_PREFIX: &str = "playbook_";

/// Prefix of a playbook's resource URI: `maidan://playbooks/<role>`.
pub const URI_PREFIX: &str = "maidan://playbooks/";

/// The resource kind in `maidan://<kind>/<id>`, as `resources.rs` routes it.
pub const RESOURCE_KIND: &str = "playbooks";

const MIME_TYPE: &str = "text/markdown";

/// Each role (the skill directory's name without `maidan-`) and its file.
const SOURCES: &[(&str, &str)] = &[
    (
        "admin",
        include_str!("../../../examples/claude-code/maidan-admin/SKILL.md"),
    ),
    (
        "approver",
        include_str!("../../../examples/claude-code/maidan-approver/SKILL.md"),
    ),
    (
        "claimant",
        include_str!("../../../examples/claude-code/maidan-claimant/SKILL.md"),
    ),
    (
        "coordinator-wake",
        include_str!("../../../examples/claude-code/maidan-coordinator-wake/SKILL.md"),
    ),
    (
        "credits",
        include_str!("../../../examples/claude-code/maidan-credits/SKILL.md"),
    ),
    (
        "dispatcher",
        include_str!("../../../examples/claude-code/maidan-dispatcher/SKILL.md"),
    ),
    (
        "inbox",
        include_str!("../../../examples/claude-code/maidan-inbox/SKILL.md"),
    ),
    (
        "memory",
        include_str!("../../../examples/claude-code/maidan-memory/SKILL.md"),
    ),
    (
        "operator",
        include_str!("../../../examples/claude-code/maidan-operator/SKILL.md"),
    ),
    (
        "planner",
        include_str!("../../../examples/claude-code/maidan-planner/SKILL.md"),
    ),
    (
        "recipes",
        include_str!("../../../examples/claude-code/maidan-recipes/SKILL.md"),
    ),
    (
        "reviewer",
        include_str!("../../../examples/claude-code/maidan-reviewer/SKILL.md"),
    ),
];

/// One role's playbook: the skill's frontmatter and body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playbook {
    /// The skill directory's name without `maidan-`, such as `claimant`.
    pub role: &'static str,
    /// The frontmatter's `name`, such as `maidan-claimant`.
    pub name: &'static str,
    /// The frontmatter's `description`.
    pub description: &'static str,
    /// Everything after the frontmatter.
    pub body: &'static str,
}

impl Playbook {
    /// The prompt's name, `playbook_<role>`.
    pub fn prompt_name(&self) -> String {
        format!("{PROMPT_PREFIX}{}", self.role)
    }

    /// The resource's URI, `maidan://playbooks/<role>`.
    pub fn uri(&self) -> String {
        format!("{URI_PREFIX}{}", self.role)
    }

    /// A human title, such as `Coordinator wake playbook`.
    pub fn title(&self) -> String {
        let role = self.role.replace('-', " ");
        let mut chars = role.chars();
        let head: String = chars
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default();
        format!("{head}{} playbook", chars.as_str())
    }

    fn prompt_entry(&self) -> Value {
        json!({
            "name": self.prompt_name(),
            "title": self.title(),
            "description": self.description,
            "arguments": [],
        })
    }

    fn resource_entry(&self) -> Value {
        json!({
            "uri": self.uri(),
            "name": self.prompt_name(),
            "title": self.title(),
            "description": self.description,
            "mimeType": MIME_TYPE,
        })
    }
}

/// Split a skill file into `(name, description, body)`. The frontmatter is the
/// block between the first two `---` lines, one `key: value` per line, which
/// is all the skills use, so no YAML parser is needed.
fn parse(text: &'static str) -> Result<(&'static str, &'static str, &'static str), &'static str> {
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().ok_or("the file is empty")?;
    if first.trim_end() != "---" {
        return Err("the file does not open with a --- frontmatter line");
    }
    let mut consumed = first.len();
    let mut name = None;
    let mut description = None;
    for line in lines {
        consumed += line.len();
        let trimmed = line.trim_end();
        if trimmed == "---" {
            let body = text
                .get(consumed..)
                .unwrap_or("")
                .trim_start_matches(['\r', '\n']);
            let name = name.filter(|v: &&str| !v.is_empty()).ok_or("no name")?;
            let description = description
                .filter(|v: &&str| !v.is_empty())
                .ok_or("no description")?;
            if body.trim().is_empty() {
                return Err("the body is empty");
            }
            return Ok((name, description, body));
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(unquote(value.trim()));
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            description = Some(unquote(value.trim()));
        }
    }
    Err("the frontmatter is never closed")
}

/// A value YAML wrote inside one pair of matching quotes, without them.
fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// Parse one source. A malformed file is a test failure
/// (`every_playbook_has_a_name_a_description_and_a_body`); at runtime
/// [`all`] skips it and logs why.
fn load(role: &'static str, text: &'static str) -> Result<Playbook, &'static str> {
    let (name, description, body) = parse(text)?;
    Ok(Playbook {
        role,
        name,
        description,
        body,
    })
}

/// Every playbook that parses, in [`SOURCES`] order.
pub fn all() -> &'static [Playbook] {
    static ALL: OnceLock<Vec<Playbook>> = OnceLock::new();
    ALL.get_or_init(|| {
        SOURCES
            .iter()
            .filter_map(|&(role, text)| match load(role, text) {
                Ok(playbook) => Some(playbook),
                Err(reason) => {
                    tracing::error!(role = %role, reason = %reason, "skipping a malformed playbook");
                    None
                }
            })
            .collect()
    })
}

/// The playbook for `role`, if there is one.
pub fn find(role: &str) -> Option<&'static Playbook> {
    all().iter().find(|playbook| playbook.role == role)
}

/// The `prompts/list` entries, one per playbook.
pub fn prompts() -> Vec<Value> {
    all().iter().map(Playbook::prompt_entry).collect()
}

/// The `resources/list` entries, one per playbook.
pub fn resources() -> Vec<Value> {
    all().iter().map(Playbook::resource_entry).collect()
}

/// `prompts/get` for a prompt named `name`, or `None` when `name` is not a
/// playbook's.
pub fn prompt(name: &str) -> Option<Value> {
    let playbook = find(name.strip_prefix(PROMPT_PREFIX)?)?;
    Some(json!({
        "description": playbook.description,
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": playbook.body }
        }]
    }))
}

/// `resources/read` of `maidan://playbooks/<role>`: the body as markdown, or
/// not found for a role that is not a playbook.
pub fn read(role: &str) -> Result<Value, McpError> {
    let playbook = find(role).ok_or(McpError::NotFound)?;
    Ok(json!({
        "contents": [{
            "uri": playbook.uri(),
            "mimeType": MIME_TYPE,
            "text": playbook.body
        }]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill_roles_on_disk() -> Vec<String> {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/claude-code");
        let mut roles = Vec::new();
        for entry in std::fs::read_dir(&dir).expect("examples/claude-code is readable") {
            let path = entry.expect("a directory entry").path();
            let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(role) = dir_name.strip_prefix("maidan-")
                && path.join("SKILL.md").is_file()
            {
                roles.push(role.to_string());
            }
        }
        roles.sort();
        roles
    }

    #[test]
    fn every_skill_on_disk_is_served_and_nothing_else() {
        let mut served: Vec<String> = all().iter().map(|p| p.role.to_string()).collect();
        served.sort();
        assert_eq!(
            served,
            skill_roles_on_disk(),
            "examples/claude-code/maidan-*/SKILL.md and playbooks::SOURCES disagree: \
             add the new skill to SOURCES, or remove the entry whose file is gone"
        );
        assert_eq!(
            SOURCES.len(),
            all().len(),
            "a source did not parse and was skipped"
        );
    }

    #[test]
    fn every_playbook_has_a_name_a_description_and_a_body() {
        for &(role, text) in SOURCES {
            let playbook = load(role, text)
                .unwrap_or_else(|reason| panic!("maidan-{role}/SKILL.md: {reason}"));
            assert!(!playbook.name.is_empty(), "{role}: name");
            assert!(!playbook.description.is_empty(), "{role}: description");
            assert!(!playbook.body.trim().is_empty(), "{role}: body");
            assert!(
                !playbook.body.starts_with("---"),
                "{role}: the body starts with the frontmatter fence"
            );
            assert_eq!(
                playbook.name,
                format!("maidan-{role}"),
                "{role}: the frontmatter name is the directory's name"
            );
        }
    }

    #[test]
    fn playbook_names_and_uris_are_unique() {
        let mut roles: Vec<&str> = all().iter().map(|p| p.role).collect();
        let mut names: Vec<&str> = all().iter().map(|p| p.name).collect();
        roles.sort_unstable();
        names.sort_unstable();
        let (role_count, name_count) = (roles.len(), names.len());
        roles.dedup();
        names.dedup();
        assert_eq!(roles.len(), role_count, "a role is listed twice");
        assert_eq!(names.len(), name_count, "a frontmatter name is used twice");
    }

    #[test]
    fn the_body_is_what_follows_the_frontmatter() {
        let text: &'static str =
            "---\nname: maidan-x\ndescription: \"Do x.\"\n---\n\n# X\n\nBody.\n";
        let (name, description, body) = parse(text).unwrap();
        assert_eq!(name, "maidan-x");
        assert_eq!(description, "Do x.");
        assert_eq!(body, "# X\n\nBody.\n");
    }

    #[test]
    fn a_file_without_closed_frontmatter_is_refused() {
        assert!(parse("# No frontmatter\n").is_err());
        assert!(parse("---\nname: a\ndescription: b\n# never closed\n").is_err());
        assert!(parse("---\nname: a\n---\nbody\n").is_err());
        assert!(parse("---\nname: a\ndescription: b\n---\n\n").is_err());
    }

    #[test]
    fn a_prompt_returns_the_body_as_one_user_message() {
        for playbook in all() {
            let got = prompt(&playbook.prompt_name()).expect("a listed playbook is served");
            assert_eq!(got["description"], playbook.description);
            let messages = got["messages"].as_array().expect("messages");
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0]["role"], "user");
            assert_eq!(messages[0]["content"]["type"], "text");
            assert_eq!(messages[0]["content"]["text"], playbook.body);
        }
        assert!(prompt("playbook_nope").is_none());
        assert!(prompt("claimant").is_none());
    }

    #[test]
    fn a_resource_read_returns_the_body_as_markdown() {
        for playbook in all() {
            let got = read(playbook.role).expect("a listed playbook is readable");
            let content = &got["contents"][0];
            assert_eq!(content["uri"], playbook.uri());
            assert_eq!(content["mimeType"], "text/markdown");
            assert_eq!(content["text"], playbook.body);
        }
        assert!(matches!(read("nope"), Err(McpError::NotFound)));
    }

    #[test]
    fn listings_name_every_playbook_with_its_description() {
        let prompts = prompts();
        let resources = resources();
        assert_eq!(prompts.len(), all().len());
        assert_eq!(resources.len(), all().len());
        for playbook in all() {
            let prompt = prompts
                .iter()
                .find(|p| p["name"] == playbook.prompt_name().as_str())
                .expect("the prompt is listed");
            assert_eq!(prompt["description"], playbook.description);
            assert_eq!(prompt["arguments"], json!([]));
            let resource = resources
                .iter()
                .find(|r| r["uri"] == playbook.uri().as_str())
                .expect("the resource is listed");
            assert_eq!(resource["name"], playbook.prompt_name().as_str());
            assert_eq!(resource["mimeType"], "text/markdown");
            assert_eq!(resource["description"], playbook.description);
        }
    }

    #[test]
    fn a_title_reads_as_the_role_and_the_word_playbook() {
        let claimant = find("claimant").expect("the claimant playbook");
        assert_eq!(claimant.title(), "Claimant playbook");
        let wake = find("coordinator-wake").expect("the coordinator-wake playbook");
        assert_eq!(wake.title(), "Coordinator wake playbook");
    }
}
