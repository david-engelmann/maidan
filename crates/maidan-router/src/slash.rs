//! Parse `/command args` from message bodies.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSlashCommand {
    pub name: String,
    pub args: String,
}

/// Parse a leading slash command from `body`.
///
/// Returns `None` when the body is not a slash command (no leading `/`, invalid
/// name, or empty name). Command names are lowercase ASCII `[a-z0-9_-]`.
pub fn parse_slash_command(body: &str) -> Option<ParsedSlashCommand> {
    let trimmed = body.trim_start();
    if !trimmed.starts_with('/') {
        return None;
    }
    let rest = &trimmed[1..];
    if rest.is_empty() {
        return None;
    }
    let mut name_end = 0;
    for (i, ch) in rest.char_indices() {
        if is_name_char(ch) {
            name_end = i + ch.len_utf8();
        } else {
            break;
        }
    }
    if name_end == 0 {
        return None;
    }
    let name = rest[..name_end].to_ascii_lowercase();
    let args = rest[name_end..].trim_start().to_string();
    Some(ParsedSlashCommand { name, args })
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
}
