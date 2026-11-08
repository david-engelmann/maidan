//! The credential guard: the last look before Maidan publishes text a
//! producer wrote.
//!
//! A review agent reads untrusted content (the diff it reviews, the issue it
//! answers), and content can carry a prompt injection that tells it to paste
//! a credential it can read into its result: an API key, a GitHub token, an
//! OAuth token. Maidan holds the sender, so without this check it would
//! publish that credential on a pull request, in a Slack channel or on a
//! Linear issue, where people it was never meant for can read it.
//!
//! Every outbound send the egress worker makes, and every write the change
//! flow makes, first passes its text through [`screen`]: a rendered comment
//! or message, a relayed projector message, a notice, the inline review and
//! the check run, and for a change the pull request title and body, the
//! commit message, the commit status and the diff's added lines. A credential
//! shape refuses the whole send, recorded the way every other refusal is (a
//! skip that no retry repeats), and nothing reaches the surface.
//!
//! **Refused, never redacted.** A redacted result is still the one the
//! injection shaped, and the operator should read the thread before anyone
//! else does.
//!
//! **The match is written nowhere.** The reason names the kind of credential,
//! the part that held it and the surface; the log line, the metric
//! (`maidan_egress_credential_refused_total{surface,kind}`) and the operator
//! alert say the same and no more.
//!
//! The shapes are hand-written matchers, each a fixed prefix and a run of the
//! characters the credential is made of, sized to keep prose and slugs out;
//! the server takes no regex dependency for a handful of prefixes.
//!
//! `MAIDAN_EGRESS_CREDENTIAL_GUARD=off` turns the guard off, for an operator
//! whose producers legitimately publish one of these shapes (a fixture key
//! in a change, say). Anything else, or unset, leaves it on.

use maidan_types::{EgressSurface, EgressTarget, ThreadId};

/// The switch that turns the guard off (`off`); on unless it says so.
pub const GUARD_ENV: &str = "MAIDAN_EGRESS_CREDENTIAL_GUARD";

/// A kind of credential the guard knows the shape of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_`, or a fine-grained `github_pat_`.
    Github,
    /// `sk-ant-…`.
    Anthropic,
    /// `sk-…`, `sk-proj-…`, `sk-svcacct-…`.
    OpenAi,
    /// `AKIA…` / `ASIA…`.
    AwsAccessKey,
    /// `xoxb-…` and the other Slack token prefixes.
    Slack,
    /// Three base64url segments, each a JSON object's encoding: OAuth access
    /// and refresh tokens are often these.
    Jwt,
    /// A PEM `-----BEGIN … PRIVATE KEY-----` header.
    PrivateKey,
    /// Maidan's own bearer, share-ticket and invite secrets (`maid_…`).
    Maidan,
}

impl CredentialKind {
    /// The metric label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Github => "github_token",
            Self::Anthropic => "anthropic_key",
            Self::OpenAi => "openai_key",
            Self::AwsAccessKey => "aws_access_key",
            Self::Slack => "slack_token",
            Self::Jwt => "jwt",
            Self::PrivateKey => "private_key",
            Self::Maidan => "maidan_token",
        }
    }

    /// What it looks like, by its prefix only: never any of the credential.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Github => "a GitHub token (gh*_…)",
            Self::Anthropic => "an Anthropic API key (sk-ant-…)",
            Self::OpenAi => "an OpenAI-style API key (sk-…)",
            Self::AwsAccessKey => "an AWS access key id (AKIA…)",
            Self::Slack => "a Slack token (xox*-…)",
            Self::Jwt => "a JSON Web Token (eyJ…)",
            // Not the header itself: a reason quoting it would be refused
            // in turn wherever it is relayed.
            Self::PrivateKey => "a PEM private key (BEGIN … PRIVATE KEY)",
            Self::Maidan => "a Maidan token (maid_…)",
        }
    }
}

/// The first credential shape in `text`, if any.
pub fn credential_kind(text: &str) -> Option<CredentialKind> {
    let bytes = text.as_bytes();
    (0..bytes.len()).find_map(|at| {
        let before = at.checked_sub(1).map(|i| bytes[i]);
        shape_at(&bytes[at..], before)
    })
}

/// Whether the guard is on: it is unless the operator wrote `off`.
pub fn enabled() -> bool {
    !std::env::var(GUARD_ENV).is_ok_and(|value| value.trim().eq_ignore_ascii_case("off"))
}

/// The first credential shape across `texts`, or `None` when the guard is
/// off. For a send that is skipped rather than refused (the inline review and
/// check run, after their summary landed), which logs on its own.
pub fn first_credential<'a>(texts: impl IntoIterator<Item = &'a str>) -> Option<CredentialKind> {
    if !enabled() {
        return None;
    }
    texts.into_iter().find_map(credential_kind)
}

/// Screen the named parts of one send to `target`. `None` means send it.
/// `Some(reason)` means refuse it: the reason is what the delivery records,
/// and it has been logged, counted and said to the operator channel.
pub async fn screen<'a>(
    target: &EgressTarget,
    thread_id: ThreadId,
    parts: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    if !enabled() {
        return None;
    }
    let (part, kind) = parts
        .into_iter()
        .find_map(|(part, text)| credential_kind(text).map(|kind| (part, kind)))?;
    let surface = target.surface();
    let reason = refusal(part, kind, surface);
    tracing::warn!(
        %thread_id,
        surface = surface.as_str(),
        kind = kind.as_str(),
        %reason,
        "egress: credential guard refused a send"
    );
    crate::metrics::record_egress_credential_refused(surface.as_str(), kind.as_str());
    crate::ops_alert::alert(
        &format!("egress-credential:{thread_id}:{target}"),
        &format!(
            "A delivery was refused: thread {thread_id} to {target}, {reason}. The producer may have been prompt-injected by what it read; look at the thread before replaying it."
        ),
    )
    .await;
    Some(reason)
}

/// The recorded reason: the part, the kind and the surface, never the match.
pub fn refusal(part: &str, kind: CredentialKind, surface: EgressSurface) -> String {
    format!(
        "refused: the {part} holds what looks like {}; nothing was sent to {}",
        kind.describe(),
        surface_name(surface)
    )
}

fn surface_name(surface: EgressSurface) -> &'static str {
    match surface {
        EgressSurface::Slack => "Slack",
        EgressSurface::Github => "GitHub",
        EgressSurface::GithubBranch => "the GitHub branch",
        EgressSurface::Linear => "Linear",
    }
}

/// The shape starting at the head of `s`, given the byte `before` it.
///
/// A prefix that also ends ordinary words (`task-…`, `mermaid_…`), and the
/// AWS id, which stands as a whole word, must not follow a letter or digit.
/// A JWT must start its run of base64url characters: besides keeping
/// `…eyJ…` inside a longer token out, that makes every JWT scan start at a
/// distinct run, so a text of repeated `eyJ` costs linear time. The other
/// prefixes are distinctive enough anywhere.
fn shape_at(s: &[u8], before: Option<u8>) -> Option<CredentialKind> {
    let after_word = before.is_some_and(alnum);
    match s.first()? {
        b'g' => github(s),
        b's' if !after_word => anthropic(s).or_else(|| openai(s)),
        b'A' if !after_word => aws(s),
        b'x' => slack(s),
        b'e' if !before.is_some_and(key_char) => jwt(s),
        b'-' => private_key(s),
        b'm' if !after_word => maidan(s),
        _ => None,
    }
}

/// How many leading bytes of `s` satisfy `ok`, counting no further than
/// `cap`. A shape that only needs a minimum length caps at it, so no scan
/// re-reads a long run from every position inside it.
fn run(s: &[u8], ok: fn(u8) -> bool, cap: usize) -> usize {
    s.iter().take(cap).take_while(|b| ok(**b)).count()
}

/// The furthest the `sk-` shape looks for its random stretch: every
/// generated key has one well inside it.
const OPENAI_SCAN: usize = 256;

fn alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn key_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn github(s: &[u8]) -> Option<CredentialKind> {
    if let Some(rest) = s.strip_prefix(b"github_pat_") {
        return (run(rest, word, 22) == 22).then_some(CredentialKind::Github);
    }
    let [b'g', b'h', kind, b'_', rest @ ..] = s else {
        return None;
    };
    (matches!(kind, b'p' | b'o' | b'u' | b's' | b'r') && run(rest, alnum, 36) == 36)
        .then_some(CredentialKind::Github)
}

fn anthropic(s: &[u8]) -> Option<CredentialKind> {
    let rest = s.strip_prefix(b"sk-ant-")?;
    (run(rest, key_char, 20) == 20).then_some(CredentialKind::Anthropic)
}

/// `sk-` and 32 or more key characters, of which one dash-free stretch of 20
/// or more mixes upper case, lower case and digits. A generated key all but
/// always has one; a slug (`sk-learn-…`, a branch name) never does.
fn openai(s: &[u8]) -> Option<CredentialKind> {
    let rest = s.strip_prefix(b"sk-")?;
    let len = run(rest, key_char, OPENAI_SCAN);
    let random = rest[..len].split(|b| *b == b'-').any(|stretch| {
        stretch.len() >= 20
            && stretch.iter().any(u8::is_ascii_uppercase)
            && stretch.iter().any(u8::is_ascii_lowercase)
            && stretch.iter().any(u8::is_ascii_digit)
    });
    (len >= 32 && random).then_some(CredentialKind::OpenAi)
}

fn aws(s: &[u8]) -> Option<CredentialKind> {
    let (prefix, id) = (s.get(..4)?, s.get(4..20)?);
    let whole_word = !s.get(20).is_some_and(|b| b.is_ascii_alphanumeric());
    ((prefix == b"AKIA" || prefix == b"ASIA")
        && id
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && whole_word)
        .then_some(CredentialKind::AwsAccessKey)
}

fn slack(s: &[u8]) -> Option<CredentialKind> {
    let [b'x', b'o', b'x', kind, b'-', rest @ ..] = s else {
        return None;
    };
    (matches!(kind, b'a' | b'b' | b'p' | b'r' | b's')
        && run(rest, |b| b.is_ascii_alphanumeric() || b == b'-', 10) == 10)
        .then_some(CredentialKind::Slack)
}

/// `eyJ` (a base64url `{"`) and 10 or more characters, twice, dot-separated,
/// then a signature of 10 or more.
fn jwt(s: &[u8]) -> Option<CredentialKind> {
    fn object(s: &[u8]) -> Option<&[u8]> {
        let rest = s.strip_prefix(b"eyJ")?;
        let len = run(rest, key_char, usize::MAX);
        (len >= 10).then(|| &rest[len..])
    }
    let rest = object(s)?.strip_prefix(b".")?;
    let rest = object(rest)?.strip_prefix(b".")?;
    (run(rest, key_char, 10) == 10).then_some(CredentialKind::Jwt)
}

/// `-----BEGIN ` and, on the same line, `PRIVATE KEY-----`: RSA, EC,
/// OPENSSH, ENCRYPTED and plain PKCS#8 headers alike.
fn private_key(s: &[u8]) -> Option<CredentialKind> {
    const TAIL: &[u8] = b"PRIVATE KEY-----";
    let rest = s.strip_prefix(b"-----BEGIN ")?;
    let line_end = rest
        .iter()
        .position(|b| *b == b'\n')
        .unwrap_or(rest.len())
        .min(64);
    rest[..line_end]
        .windows(TAIL.len())
        .any(|window| window == TAIL)
        .then_some(CredentialKind::PrivateKey)
}

/// `maid_`, `maid_share_` or `maid_inv_` and 32 or more lowercase hex digits
/// (minted secrets carry 64).
fn maidan(s: &[u8]) -> Option<CredentialKind> {
    let rest = s.strip_prefix(b"maid_")?;
    let rest = rest
        .strip_prefix(b"share_")
        .or_else(|| rest.strip_prefix(b"inv_"))
        .unwrap_or(rest);
    (run(
        rest,
        |b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        32,
    ) == 32)
        .then_some(CredentialKind::Maidan)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every credential here is assembled at run time, so the repository holds
    // no string a secrets scanner reads as a live key.
    fn repeat(c: char, n: usize) -> String {
        std::iter::repeat_n(c, n).collect()
    }

    /// 40 characters that mix case and digits, as a generated key does.
    fn mixed(n: usize) -> String {
        "aB3dE5gH7jK9mN1pQ2rS4tU6vW8xY0zA"
            .chars()
            .cycle()
            .take(n)
            .collect()
    }

    fn kind(text: &str) -> Option<&'static str> {
        credential_kind(text).map(CredentialKind::as_str)
    }

    #[test]
    fn a_github_token_of_every_prefix_is_caught_and_a_short_one_is_not() {
        for prefix in ["ghp", "gho", "ghu", "ghs", "ghr"] {
            let token = format!("{prefix}_{}", mixed(36));
            assert_eq!(kind(&format!("token: {token}")), Some("github_token"));
            let short = format!("{prefix}_{}", mixed(35));
            assert_eq!(kind(&format!("token: {short}")), None, "{prefix}");
        }
        assert_eq!(kind(&format!("ghx_{}", mixed(40))), None);
    }

    #[test]
    fn a_fine_grained_github_token_is_caught_and_a_short_one_is_not() {
        let token = format!("github_pat_{}_{}", mixed(11), mixed(40));
        assert_eq!(kind(&token), Some("github_token"));
        assert_eq!(kind(&format!("github_pat_{}", mixed(21))), None);
    }

    #[test]
    fn an_anthropic_key_is_caught_and_a_short_one_is_not() {
        let key = format!("sk-ant-api03-{}", mixed(40));
        assert_eq!(kind(&format!("export KEY={key}")), Some("anthropic_key"));
        assert_eq!(kind("see sk-ant-short-1"), None);
    }

    #[test]
    fn an_openai_key_is_caught_but_short_or_slug_shaped_text_is_not() {
        assert_eq!(kind(&format!("sk-{}", mixed(48))), Some("openai_key"));
        assert_eq!(
            kind(&format!("sk-proj-{}-{}", mixed(30), mixed(40))),
            Some("openai_key")
        );
        assert_eq!(
            kind(&format!("sk-svcacct-{}", mixed(60))),
            Some("openai_key")
        );
        assert_eq!(kind("sk-1"), None);
        assert_eq!(
            kind("sk-learn-is-a-python-library-for-machine-learning-2024"),
            None
        );
        assert_eq!(kind(&format!("sk-{}", repeat('a', 48))), None);
        assert_eq!(kind(&format!("task-{}", mixed(48))), None, "inside a word");
    }

    #[test]
    fn an_aws_access_key_id_is_caught_only_as_a_whole_word() {
        let id = format!("AKIA{}", "IOSFODNN7EXAMPL1");
        assert_eq!(kind(&format!("id={id}")), Some("aws_access_key"));
        let session = format!("ASIA{}", "Q2W3E4R5T6Y7U8I9");
        assert_eq!(kind(&session), Some("aws_access_key"));
        assert_eq!(kind(&format!("{id}X")), None, "seventeen characters");
        assert_eq!(kind(&format!("X{id}")), None, "inside a word");
        assert_eq!(kind(&format!("AKIA{}", "iosfodnn7exampl1")), None);
    }

    #[test]
    fn a_slack_token_is_caught_and_a_lookalike_is_not() {
        for prefix in ["xoxb", "xoxp", "xoxa", "xoxr", "xoxs"] {
            let token = format!("{prefix}-1234567890-{}", mixed(24));
            assert_eq!(kind(&token), Some("slack_token"), "{prefix}");
        }
        assert_eq!(kind("xoxb-123"), None);
        assert_eq!(kind(&format!("xoxz-{}", mixed(24))), None);
    }

    #[test]
    fn a_jwt_is_caught_and_two_segments_are_not() {
        let jwt = format!("eyJ{}.eyJ{}.{}", mixed(20), mixed(30), mixed(43));
        assert_eq!(kind(&format!("Bearer {jwt}")), Some("jwt"));
        let unsigned = format!("eyJ{}.eyJ{}", mixed(20), mixed(30));
        assert_eq!(kind(&unsigned), None);
        let short = format!("eyJ{}.eyJ{}.{}", mixed(9), mixed(30), mixed(43));
        assert_eq!(kind(&short), None);
        assert_eq!(kind(&format!("x-{jwt}")), None, "inside a longer token");
    }

    #[test]
    fn a_text_of_repeated_prefixes_is_scanned_in_linear_time() {
        // An uncapped scan from every prefix would read about a billion
        // bytes here; the capped one reads a few million.
        for prefix in ["sk-", "eyJ", "ghp_", "xoxb-", "maid_"] {
            let text = prefix.repeat(20_000);
            let started = std::time::Instant::now();
            assert_eq!(kind(&text), None, "{prefix}");
            assert!(started.elapsed().as_secs() < 5, "{prefix}");
        }
    }

    #[test]
    fn a_private_key_header_is_caught_and_a_public_one_is_not() {
        let dashes = repeat('-', 5);
        for label in ["RSA PRIVATE KEY", "PRIVATE KEY", "OPENSSH PRIVATE KEY"] {
            let pem = format!("+{dashes}BEGIN {label}{dashes}\nMIIE...\n");
            assert_eq!(kind(&pem), Some("private_key"), "{label}");
        }
        assert_eq!(kind(&format!("{dashes}BEGIN PUBLIC KEY{dashes}")), None);
        assert_eq!(
            kind(&format!(
                "{dashes}BEGIN CERTIFICATE{dashes}\nPRIVATE KEY{dashes}"
            )),
            None,
            "the tail must be on the header's line"
        );
    }

    #[test]
    fn maidans_own_secrets_are_caught_and_a_short_one_is_not() {
        let hex = "0123456789abcdef".repeat(4);
        for prefix in ["maid_", "maid_share_", "maid_inv_"] {
            assert_eq!(kind(&format!("{prefix}{hex}")), Some("maidan_token"));
        }
        assert_eq!(kind("maid_test"), None);
        assert_eq!(kind(&format!("mermaid_{hex}")), None, "inside a word");
    }

    #[test]
    fn ordinary_review_prose_holds_no_credential() {
        let prose = "## Findings\n- `src/auth.rs:42` the token is logged at debug; \
                     use `TokenSecret` (its Debug is redacted).\n- task-1234: rename \
                     `sk_live` to `api_key`. See https://github.com/acme/widgets/pull/7 \
                     and commit 0123456789abcdef0123456789abcdef01234567.";
        assert_eq!(credential_kind(prose), None);
    }

    #[test]
    fn no_refusal_reason_trips_the_guard_when_it_is_relayed() {
        let kinds = [
            CredentialKind::Github,
            CredentialKind::Anthropic,
            CredentialKind::OpenAi,
            CredentialKind::AwsAccessKey,
            CredentialKind::Slack,
            CredentialKind::Jwt,
            CredentialKind::PrivateKey,
            CredentialKind::Maidan,
        ];
        for kind in kinds {
            let reason = refusal("diff", kind, EgressSurface::GithubBranch);
            assert_eq!(credential_kind(&reason), None, "{reason}");
        }
    }

    #[test]
    fn the_reason_names_the_kind_and_surface_but_never_the_match() {
        let token = format!("ghp_{}", mixed(36));
        let found = credential_kind(&token).unwrap();
        let reason = refusal("comment", found, EgressSurface::Github);
        assert_eq!(
            reason,
            "refused: the comment holds what looks like a GitHub token (gh*_…); nothing was sent to GitHub"
        );
        assert!(!reason.contains(&token));
    }
}
