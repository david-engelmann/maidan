//! Parse `/command args` from message bodies.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSlashCommand {
    pub name: String,
    pub args: String,
}

const MAX_COMMAND_NAME_LEN: usize = 64;

/// Parse a leading slash command from `body`.
///
/// Returns `None` when the body is not a slash command. The name is 1 to 64
/// ASCII `[A-Za-z0-9_-]` characters, starts with a letter or digit, and ends
/// at the end of the text or at whitespace. `/etc/hosts`, `/v1.2.0`, and a
/// markdown rule are ordinary text. The stored name is lowercased.
pub fn parse_slash_command(body: &str) -> Option<ParsedSlashCommand> {
    let trimmed = body.trim_start();
    let rest = trimmed.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let mut name_end = 0;
    for (i, ch) in rest.char_indices() {
        if is_name_char(ch) {
            name_end = i + ch.len_utf8();
            if name_end > MAX_COMMAND_NAME_LEN {
                return None;
            }
        } else {
            break;
        }
    }
    if name_end == 0 {
        return None;
    }
    let name = &rest[..name_end];
    if !name.as_bytes()[0].is_ascii_alphanumeric() {
        return None;
    }
    let tail = &rest[name_end..];
    if !(tail.is_empty() || tail.starts_with(char::is_whitespace)) {
        return None;
    }
    Some(ParsedSlashCommand {
        name: name.to_ascii_lowercase(),
        args: tail.trim_start().to_string(),
    })
}

fn is_name_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_name_and_args() {
        let parsed = parse_slash_command("/deploy staging").expect("parse");
        assert_eq!(parsed.name, "deploy");
        assert_eq!(parsed.args, "staging");
    }

    #[test]
    fn normalizes_command_name_case() {
        let parsed = parse_slash_command("/DePloy").expect("parse");
        assert_eq!(parsed.name, "deploy");
        assert_eq!(parsed.args, "");
    }

    #[test]
    fn ignores_non_slash_bodies() {
        assert!(parse_slash_command("hello /deploy").is_none());
        assert!(parse_slash_command("").is_none());
    }

    #[test]
    fn rejects_invalid_name() {
        assert!(parse_slash_command("/").is_none());
        assert!(parse_slash_command("/@bad").is_none());
    }

    #[test]
    fn a_path_url_version_or_rule_is_not_a_command() {
        for body in [
            "/etc/hosts is down",
            "/usr/bin/env",
            "/usr/local/bin",
            "/v1.2.0",
            "/-flag",
            "/---",
            "//comment",
            "/foo@bar",
        ] {
            assert!(parse_slash_command(body).is_none(), "{body}");
        }
        let long = format!("/{}", "a".repeat(MAX_COMMAND_NAME_LEN + 1));
        assert!(parse_slash_command(&long).is_none());
    }

    #[test]
    fn a_command_may_end_at_space_tab_or_newline() {
        let parsed = parse_slash_command("/deploy").expect("bare");
        assert_eq!(parsed.name, "deploy");
        assert_eq!(parsed.args, "");

        let parsed = parse_slash_command("/deploy staging").expect("space");
        assert_eq!(parsed.name, "deploy");
        assert_eq!(parsed.args, "staging");

        let parsed = parse_slash_command("/deploy\tstaging").expect("tab");
        assert_eq!(parsed.name, "deploy");
        assert_eq!(parsed.args, "staging");

        let parsed = parse_slash_command("/deploy\nstaging").expect("newline");
        assert_eq!(parsed.name, "deploy");
        assert_eq!(parsed.args, "staging");
    }
}
