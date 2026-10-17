//! Database dialect detection. Used by the server to choose which
//! concrete `Store` impl + migration runner to instantiate.

use crate::error::StoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Postgres,
    Sqlite,
}

impl Dialect {
    /// Detect the dialect from a connection string. Recognizes
    /// `postgres://`, `postgresql://`, and `sqlite://` prefixes.
    pub fn from_url(url: &str) -> Result<Self, StoreError> {
        if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Ok(Self::Postgres)
        } else if url.starts_with("sqlite:") {
            Ok(Self::Sqlite)
        } else {
            // This error reaches the boot failure and the logs, and the URL is
            // most likely to be mistyped exactly when it carries credentials.
            Err(StoreError::InvalidInput(format!(
                "cannot detect dialect from connection URL '{}': expected postgres://, postgresql://, or sqlite:",
                redact_connection_url(url)
            )))
        }
    }
}

const MASK: &str = "***";

/// `url` with its credentials masked, for error messages, logs and `Debug`:
/// the userinfo password, and the value of every query parameter (or libpq
/// `key=value` keyword) whose name looks like a credential. The scheme, user,
/// host, path and other parameters stay, so the message still says what was
/// wrong. It never parses strictly: the URLs that end up in errors are the
/// malformed ones, and those must not come back verbatim either.
pub fn redact_connection_url(url: &str) -> String {
    let Some(authority_start) = authority_start(url) else {
        // No scheme: possibly a libpq keyword string, `host=db password=x`.
        return redact_pairs(url);
    };
    let mut out = String::with_capacity(url.len());
    out.push_str(&url[..authority_start]);
    let mut rest = &url[authority_start..];
    // The userinfo ends at the last `@`, so an unencoded `@`, `/`, `?` or `#`
    // inside a password still counts as password. An `@` further on (say, in
    // a query value) only widens what is masked, never what is shown.
    if let Some(at) = rest.rfind('@') {
        let userinfo = &rest[..at];
        match userinfo.find(':') {
            Some(colon) => {
                out.push_str(&userinfo[..colon]);
                out.push(':');
                out.push_str(MASK);
            }
            None => out.push_str(userinfo),
        }
        out.push('@');
        rest = &rest[at + 1..];
    }
    out.push_str(&redact_pairs(rest));
    out
}

/// Where the part after the scheme starts: after `scheme://`, or after a
/// mistyped `scheme:` or `scheme:/`, which must not slip past the userinfo
/// masking. `None` when the text before the first `:` is not a scheme, as in
/// `host=db:5432 password=x`.
fn authority_start(url: &str) -> Option<usize> {
    let colon = url.find(':')?;
    let scheme = &url[..colon];
    let is_scheme = !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    if !is_scheme {
        return None;
    }
    let slashes = url[colon + 1..].chars().take_while(|c| *c == '/').count();
    Some(colon + 1 + slashes)
}

/// Mask the value of every credential-named `key=value` pair, keeping the
/// separators (`?`, `&`, `;`, whitespace) exactly as they were.
fn redact_pairs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pair = String::new();
    for c in s.chars() {
        if matches!(c, '?' | '&' | ';') || c.is_whitespace() {
            out.push_str(&redact_pair(&pair));
            pair.clear();
            out.push(c);
        } else {
            pair.push(c);
        }
    }
    out.push_str(&redact_pair(&pair));
    out
}

fn redact_pair(pair: &str) -> String {
    match pair.split_once('=') {
        Some((key, _)) if is_credential_key(key) => format!("{key}={MASK}"),
        _ => pair.to_string(),
    }
}

/// Parameter names that carry a secret on the hosts people point
/// `DATABASE_URL` at: libpq's `password`, libSQL's `authToken`, and the
/// `token`, `key` and `secret` spellings of the others. Matched loosely
/// (case, `_` and `-` ignored) because a leak costs more than a masked
/// harmless value.
fn is_credential_key(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    const CONTAINS: [&str; 8] = [
        "password",
        "passwd",
        "pwd",
        "token",
        "secret",
        "apikey",
        "accesskey",
        "credential",
    ];
    CONTAINS.iter().any(|needle| key.contains(needle))
        || matches!(key.as_str(), "key" | "auth" | "sig" | "signature")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRETS: [&str; 2] = ["S3cr3tPw", "tok_S3cr3t"];

    fn assert_no_secret(shown: &str) {
        for secret in SECRETS {
            assert!(!shown.contains(secret), "{secret} leaked into: {shown}");
        }
    }

    // The boot error for a mistyped prefix is exactly where an operator's
    // credentials used to be printed back.
    #[test]
    fn an_unknown_prefix_error_never_shows_the_password_or_the_auth_token() {
        for url in [
            "postgress://maidan:S3cr3tPw@db.example:5432/maidan",
            "libsql://maidan-demo.turso.io?authToken=tok_S3cr3t",
            "mysql://maidan:S3cr3tPw@db.example/maidan?sslmode=require&authToken=tok_S3cr3t",
        ] {
            let err = Dialect::from_url(url).expect_err("unknown prefix");
            let shown = err.to_string();
            assert_no_secret(&shown);
            assert!(
                shown.contains("cannot detect dialect"),
                "the error still says what is wrong: {shown}"
            );
            let scheme = &url[..url.find("://").expect("scheme")];
            assert!(
                shown.contains(scheme),
                "the error still shows {scheme}: {shown}"
            );
        }
    }

    #[test]
    fn the_password_is_masked_and_the_rest_of_the_url_kept() {
        assert_eq!(
            redact_connection_url(
                "postgres://maidan:S3cr3tPw@db.example:5432/maidan?sslmode=require"
            ),
            "postgres://maidan:***@db.example:5432/maidan?sslmode=require"
        );
    }

    #[test]
    fn a_password_containing_an_at_sign_is_masked_whole() {
        let shown = redact_connection_url("postgres://maidan:S3cr@3tPw@db.example/maidan");
        assert_eq!(shown, "postgres://maidan:***@db.example/maidan");
    }

    #[test]
    fn a_password_containing_a_question_mark_is_masked_whole() {
        let shown = redact_connection_url("postgres://maidan:S3cr?3tPw@db.example/maidan");
        assert_eq!(shown, "postgres://maidan:***@db.example/maidan");
    }

    #[test]
    fn a_password_containing_a_slash_is_masked_whole() {
        let shown = redact_connection_url("postgres://maidan:S3cr/3tPw@db.example/maidan");
        assert_eq!(shown, "postgres://maidan:***@db.example/maidan");
    }

    #[test]
    fn an_at_sign_in_a_query_value_masks_more_but_leaks_nothing() {
        let shown = redact_connection_url(
            "postgres://maidan:S3cr3tPw@db.example/maidan?application_name=ops@example&password=S3cr3tPw",
        );
        assert_no_secret(&shown);
        assert!(shown.starts_with("postgres://maidan:***@"), "{shown}");
    }

    #[test]
    fn credential_query_parameters_are_masked_in_any_spelling() {
        let shown = redact_connection_url(
            "libsql://d.turso.io?authToken=tok_S3cr3t&password=S3cr3tPw&API_KEY=S3cr3tPw\
             &x-secret=S3cr3tPw&key=S3cr3tPw&tls=1",
        );
        assert_no_secret(&shown);
        assert_eq!(
            shown,
            "libsql://d.turso.io?authToken=***&password=***&API_KEY=***&x-secret=***&key=***&tls=1"
        );
    }

    #[test]
    fn a_mistyped_scheme_separator_still_masks_the_password() {
        for url in [
            "postgress:/maidan:S3cr3tPw@db.example/maidan",
            "postgres:maidan:S3cr3tPw@db.example/maidan",
        ] {
            let shown = redact_connection_url(url);
            assert_no_secret(&shown);
            assert!(shown.contains("maidan:***@db.example"), "{shown}");
        }
    }

    #[test]
    fn a_libpq_keyword_string_with_a_port_has_its_password_masked() {
        let shown = redact_connection_url("host=db.example:5432 password=S3cr@3tPw");
        assert_no_secret(&shown);
        assert_eq!(shown, "host=db.example:5432 password=***");
    }

    #[test]
    fn a_libpq_keyword_string_has_its_password_masked() {
        let shown =
            redact_connection_url("host=db.example user=maidan password=S3cr3tPw dbname=maidan");
        assert_eq!(
            shown,
            "host=db.example user=maidan password=*** dbname=maidan"
        );
    }

    #[test]
    fn urls_without_credentials_are_unchanged() {
        for url in [
            "sqlite::memory:",
            "sqlite:///var/lib/maidan/maidan.db?mode=rwc",
            "postgres://maidan@db.example/maidan",
            "postgres://db.example/maidan?sslmode=verify-full",
        ] {
            assert_eq!(redact_connection_url(url), url);
        }
    }

    #[test]
    fn known_prefixes_still_detect() {
        assert_eq!(
            Dialect::from_url("postgres://a@b/c").ok(),
            Some(Dialect::Postgres)
        );
        assert_eq!(
            Dialect::from_url("postgresql://a@b/c").ok(),
            Some(Dialect::Postgres)
        );
        assert_eq!(
            Dialect::from_url("sqlite::memory:").ok(),
            Some(Dialect::Sqlite)
        );
    }
}
