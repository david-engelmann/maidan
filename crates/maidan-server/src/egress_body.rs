//! What a delivery body actually says on each surface (Cluster 378.3). Pure — no
//! state, no I/O, no types from the result envelope, so every rule here is
//! unit-testable and the Cluster-379 delivery step composes rather than decides.
//!
//! Three problems, one per function group:
//!
//! 1. **Mentions.** A result's `rendered` is producer-authored but *quotes
//!    attacker-influenced code*: a PR body, a diff, a filename. An `@org/team`
//!    reaching GitHub or a `<!channel>` reaching Slack pings real humans from
//!    bytes an attacker chose. Maidan defuses them at the egress boundary, so
//!    producers do not have to and a producer that forgets cannot hurt anyone.
//! 2. **Size.** GitHub refuses a comment body over 65536 characters, and a 4 KB
//!    `rendered` is the *normal* case for a code review, not the pathological
//!    one. Truncation is explicit and keeps the backlink, because a body that
//!    silently lost its tail is worse than one that says it did.
//! 3. **Slack is not GitHub.** Slack speaks mrkdwn, not GFM: `*bold*` not
//!    `**bold**`, `<url|text>` not `[text](url)`, no headings, no tables. Posting
//!    `rendered` to Slack verbatim ships visibly broken output, which is why
//!    Slack receives `summary` + a compact digest + the link instead — see
//!    `docs/Result Delivery.md`.
//!
//! **The narrowness of [`gfm_to_mrkdwn`] is deliberate and load-bearing.** It is
//! not a Markdown engine; it converts the two constructs that are outright
//! *broken* rather than merely plain on Slack (links and headings), plus bold.
//! Everything else is left alone, because the rule that Slack gets a short
//! producer-written `summary` — never `rendered` — is what keeps the job small. A
//! half-correct full converter applied to 4 KB of GFM would be worse than an
//! honest narrow one applied to one line.

/// GitHub's hard ceiling on an issue/PR comment body, in characters.
pub const GITHUB_BODY_MAX_CHARS: usize = 65536;

/// What a truncated body says where the rest used to be.
const TRUNCATION_NOTICE: &str = "\n\n_…truncated by Maidan._";

/// A span of the input and whether it is code (a code span or fenced block), in
/// which case every rule here leaves it alone: a mention inside code already
/// does not notify on either surface, and rewriting code would corrupt the thing
/// the reader is trying to read.
#[derive(Debug, PartialEq, Eq)]
struct Segment<'a> {
    text: &'a str,
    code: bool,
}

/// Split Markdown into alternating prose and code segments.
///
/// Handles fenced blocks (a line-leading run of 3+ backticks or tildes, closed
/// by a run at least as long) and inline code spans (a run of N backticks closed
/// by a run of exactly N). An *unclosed* opener runs to end of input and is
/// treated as code — the conservative direction, since the alternative is
/// rewriting text the renderer will show verbatim.
fn segments(md: &str) -> Vec<Segment<'_>> {
    let bytes = md.as_bytes();
    let mut out: Vec<Segment<'_>> = Vec::new();
    let mut prose_start = 0usize;
    let mut i = 0usize;

    let at_line_start = |pos: usize| pos == 0 || bytes[pos - 1] == b'\n';
    let run_len = |pos: usize, ch: u8| {
        let mut n = 0;
        while pos + n < bytes.len() && bytes[pos + n] == ch {
            n += 1;
        }
        n
    };

    while i < bytes.len() {
        let ch = bytes[i];
        if ch != b'`' && ch != b'~' {
            i += 1;
            continue;
        }
        let fence = run_len(i, ch);
        // A fenced block: 3+ delimiters starting a line. Its closer is a run of
        // at least the same length, also at a line start.
        if fence >= 3 && at_line_start(i) {
            let mut j = i + fence;
            let end = loop {
                if j >= bytes.len() {
                    break bytes.len();
                }
                if bytes[j] == ch && at_line_start(j) && run_len(j, ch) >= fence {
                    break (j + run_len(j, ch)).min(bytes.len());
                }
                j += 1;
            };
            if prose_start < i {
                out.push(Segment {
                    text: &md[prose_start..i],
                    code: false,
                });
            }
            out.push(Segment {
                text: &md[i..end],
                code: true,
            });
            i = end;
            prose_start = end;
            continue;
        }
        // An inline code span: a run of N backticks closed by a run of exactly N.
        // Tildes are strikethrough inline, not code, so they only matter as fences.
        if ch == b'`' {
            let mut j = i + fence;
            let mut end = None;
            while j < bytes.len() {
                if bytes[j] == b'`' {
                    let closing = run_len(j, b'`');
                    if closing == fence {
                        end = Some(j + closing);
                        break;
                    }
                    j += closing;
                    continue;
                }
                j += 1;
            }
            let end = end.unwrap_or(bytes.len());
            if prose_start < i {
                out.push(Segment {
                    text: &md[prose_start..i],
                    code: false,
                });
            }
            out.push(Segment {
                text: &md[i..end],
                code: true,
            });
            i = end;
            prose_start = end;
            continue;
        }
        i += fence.max(1);
    }
    if prose_start < md.len() {
        out.push(Segment {
            text: &md[prose_start..],
            code: false,
        });
    }
    out
}

/// Rewrite only the prose segments of `md`, leaving code untouched.
fn map_prose(md: &str, f: impl Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(md.len());
    for seg in segments(md) {
        if seg.code {
            out.push_str(seg.text);
        } else {
            out.push_str(&f(seg.text));
        }
    }
    out
}

/// A GitHub username character. Usernames are alphanumeric plus hyphens; a team
/// slug after `/` also allows `.` and `_`.
fn is_login_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Defuse GitHub mention sequences in prose by wrapping them in a code span.
///
/// **Why a code span rather than an invisible character.** GitHub documents that
/// a mention inside code does not notify, so the guarantee rests on a rendering
/// rule rather than on a zero-width space or an empty HTML comment surviving
/// whatever the sanitizer does this year. It is also *visible*: a reader sees
/// that the mention was defused instead of wondering why `@someone` never
/// answered.
///
/// Follows GitHub's own grammar closely enough to leave ordinary text alone: an
/// `@` immediately after a word character is not a mention (so `user@host` and
/// `a@b.com` are untouched), and `@` must be followed by an alphanumeric.
/// Over-matching here costs a pair of backticks; under-matching pings a stranger.
pub fn neutralize_github_mentions(md: &str) -> String {
    map_prose(md, |prose| {
        let chars: Vec<char> = prose.chars().collect();
        let mut out = String::with_capacity(prose.len());
        let mut i = 0usize;
        while i < chars.len() {
            if chars[i] != '@' {
                out.push(chars[i]);
                i += 1;
                continue;
            }
            // `@` right after a word character is an email-ish infix, not a mention.
            let preceded_by_word = i > 0 && (chars[i - 1].is_ascii_alphanumeric());
            let mut end = i + 1;
            while end < chars.len() && is_login_char(chars[end]) {
                end += 1;
            }
            let login_len = end - (i + 1);
            let starts_with_alnum = chars.get(i + 1).is_some_and(|c| c.is_ascii_alphanumeric());
            if preceded_by_word || login_len == 0 || !starts_with_alnum {
                out.push('@');
                i += 1;
                continue;
            }
            // An optional `/team` suffix, so `@org/team` is defused whole rather
            // than leaving `/team` dangling outside the code span.
            let mut team_end = end;
            if chars.get(end) == Some(&'/') {
                let mut k = end + 1;
                while k < chars.len()
                    && (is_login_char(chars[k]) || chars[k] == '.' || chars[k] == '_')
                {
                    k += 1;
                }
                if k > end + 1 {
                    team_end = k;
                }
            }
            out.push('`');
            out.extend(&chars[i..team_end]);
            out.push('`');
            i = team_end;
        }
        out
    })
}

/// Defuse Slack mention sequences by escaping the opening `<`.
///
/// Slack unescapes `&lt;` back to a literal `<` when rendering, so the reader
/// still sees `<!channel>` — it simply does not broadcast. Covers channel-wide
/// broadcasts (`<!channel>`, `<!here>`, `<!everyone>`), user pings (`<@U…>`) and
/// user-group pings (`<!subteam^S…>`), because all of them share the `<!` / `<@`
/// opener. A plain `<https://…>` autolink is left alone.
pub fn neutralize_slack_mentions(text: &str) -> String {
    map_prose(text, |prose| {
        let mut out = String::with_capacity(prose.len());
        let mut chars = prose.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '<' && matches!(chars.peek(), Some('!') | Some('@')) {
                out.push_str("&lt;");
            } else {
                out.push(c);
            }
        }
        out
    })
}

/// Fit `body` (plus an optional backlink tail) inside `max` characters.
///
/// The tail is appended when it fits; a body that needs cutting keeps the tail
/// and an explicit notice, so the reader is told the content was cut *and* still
/// has the link to the whole thing. Cuts on a character boundary, and prefers the
/// last blank line or newline in the final 20% of the budget so the cut lands
/// between paragraphs rather than mid-word.
pub fn truncate_with_tail(body: &str, max: usize, tail: Option<&str>) -> String {
    let tail = tail.unwrap_or("");
    let full_len = body.chars().count() + tail.chars().count();
    if full_len <= max {
        return format!("{body}{tail}");
    }
    let reserved = tail.chars().count() + TRUNCATION_NOTICE.chars().count();
    // Nothing sensible to say in the space left: return the notice and the tail
    // rather than a body fragment that claims to be a review.
    if reserved >= max {
        let mut out = String::new();
        out.push_str(TRUNCATION_NOTICE.trim_start());
        out.push_str(tail);
        return out.chars().take(max).collect();
    }
    let budget = max - reserved;
    let kept: String = body.chars().take(budget).collect();
    let floor = budget * 4 / 5;
    let cut = kept
        .rfind("\n\n")
        .filter(|idx| kept[..*idx].chars().count() >= floor)
        .or_else(|| {
            kept.rfind('\n')
                .filter(|idx| kept[..*idx].chars().count() >= floor)
        })
        .unwrap_or(kept.len());
    format!("{}{TRUNCATION_NOTICE}{tail}", kept[..cut].trim_end())
}

/// Project GFM onto Slack mrkdwn — **narrowly**, see the module docs.
///
/// Converts what Slack renders *wrongly*:
/// - `[text](url)` → `<url|text>` (Slack shows the GFM form literally)
/// - `## Heading` → `*Heading*` (Slack has no headings)
/// - `**bold**` / `__bold__` → `*bold*` (Slack's bold is single-asterisk)
///
/// Deliberately left alone: single `*`/`_` emphasis (rewriting `*` would mangle
/// `2 * 3`, and `_x_` is already italic in mrkdwn), lists, block quotes, code
/// (fences and spans render on Slack), and tables — which do not render, and are
/// a reason to send `summary` rather than `rendered` rather than a reason to
/// grow this function.
pub fn gfm_to_mrkdwn(md: &str) -> String {
    map_prose(md, |prose| {
        let with_links = rewrite_links(prose);
        let mut out = String::with_capacity(with_links.len());
        for (idx, line) in with_links.split('\n').enumerate() {
            if idx > 0 {
                out.push('\n');
            }
            out.push_str(&rewrite_line(line));
        }
        out
    })
}

/// `[text](url)` → `<url|text>`. Skips an image (`![alt](url)`) and anything
/// whose label or target contains a nested bracket or parenthesis, since those
/// need a real parser and mis-rewriting a link is worse than leaving it readable.
fn rewrite_links(prose: &str) -> String {
    let chars: Vec<char> = prose.chars().collect();
    let mut out = String::with_capacity(prose.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] != '[' || (i > 0 && chars[i - 1] == '!') {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let label_end = (i + 1..chars.len()).find(|&k| matches!(chars[k], ']' | '[' | '\n'));
        let Some(label_end) = label_end.filter(|&k| chars[k] == ']') else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        if chars.get(label_end + 1) != Some(&'(') {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let url_end =
            (label_end + 2..chars.len()).find(|&k| matches!(chars[k], ')' | '(' | '\n' | ' '));
        let Some(url_end) = url_end.filter(|&k| chars[k] == ')') else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        let label: String = chars[i + 1..label_end].iter().collect();
        let url: String = chars[label_end + 2..url_end].iter().collect();
        if label.is_empty() || url.is_empty() {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        out.push('<');
        out.push_str(&url);
        out.push('|');
        out.push_str(&label);
        out.push('>');
        i = url_end + 1;
    }
    out
}

/// Per-line rewrites: an ATX heading becomes a bold line, and doubled emphasis
/// delimiters collapse to Slack's single asterisk.
fn rewrite_line(line: &str) -> String {
    let trimmed = line.trim_start();
    if let Some(rest) = trimmed.strip_prefix('#') {
        let hashes = 1 + rest.chars().take_while(|&c| c == '#').count();
        let title = trimmed[hashes..].trim();
        if hashes <= 6 && !title.is_empty() {
            return format!("*{}*", collapse_bold(title));
        }
    }
    collapse_bold(line)
}

/// `**x**` and `__x__` → `*x*`. Runs of exactly two delimiters only, so a single
/// `*` or `_` is preserved and `***x***` is left for a human to look at.
fn collapse_bold(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' || c == '_' {
            let run = chars[i..].iter().take_while(|&&x| x == c).count();
            if run == 2 {
                out.push('*');
                i += 2;
                continue;
            }
            out.extend(std::iter::repeat_n(c, run));
            i += run;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The body for a GitHub issue/PR comment: the producer's `rendered` GFM with
/// mentions defused, plus the backlink, truncated to GitHub's ceiling.
pub fn github_comment_body(rendered: &str, backlink: Option<&str>) -> String {
    let safe = neutralize_github_mentions(rendered);
    let tail = backlink.map(|url| format!("\n\n[View in the producer]({url})"));
    truncate_with_tail(&safe, GITHUB_BODY_MAX_CHARS, tail.as_deref())
}

/// The body for a Slack message: the one-line `summary`, a compact digest, and
/// the link — **never** `rendered`, which is GFM and would arrive visibly broken.
/// Mentions are defused and the prose is projected onto mrkdwn.
pub fn slack_message_body(summary: &str, digest: &[String], backlink: Option<&str>) -> String {
    let mut body = gfm_to_mrkdwn(summary.trim());
    for line in digest {
        body.push('\n');
        body.push_str(&gfm_to_mrkdwn(line.trim()));
    }
    if let Some(url) = backlink {
        body.push_str(&format!("\n<{url}|View in the producer>"));
    }
    neutralize_slack_mentions(&body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_mention_in_prose_is_defused_and_one_in_code_is_left_alone() {
        assert_eq!(
            neutralize_github_mentions("ping @octocat about this"),
            "ping `@octocat` about this"
        );
        // A team ping is the loudest one, and must be defused whole.
        assert_eq!(
            neutralize_github_mentions("cc @acme/platform now"),
            "cc `@acme/platform` now"
        );
        // Already inside code: it never notified, so don't touch it.
        assert_eq!(
            neutralize_github_mentions("see `@octocat` here"),
            "see `@octocat` here"
        );
        assert_eq!(
            neutralize_github_mentions("```\n@octocat\n```"),
            "```\n@octocat\n```"
        );
        // The realistic shape: a fenced diff quoting attacker-chosen bytes,
        // with prose around it.
        assert_eq!(
            neutralize_github_mentions("hi @a\n```js\n// @b\n```\nbye @c"),
            "hi `@a`\n```js\n// @b\n```\nbye `@c`"
        );
    }

    #[test]
    fn an_at_sign_that_is_not_a_mention_is_untouched() {
        for text in [
            "mail a@b.com please",
            "user@host",
            "an @ on its own",
            "@ leading space",
            "@-startswithhyphen",
            "cost is 5 @ each",
        ] {
            assert_eq!(
                neutralize_github_mentions(text),
                text,
                "expected {text:?} to be left alone"
            );
        }
    }

    #[test]
    fn slack_broadcasts_and_pings_are_escaped_but_autolinks_are_not() {
        assert_eq!(
            neutralize_slack_mentions("heads up <!channel> and <!here>"),
            "heads up &lt;!channel> and &lt;!here>"
        );
        assert_eq!(
            neutralize_slack_mentions("<@U123> and <!subteam^S1>"),
            "&lt;@U123> and &lt;!subteam^S1>"
        );
        // An autolink is not a ping.
        assert_eq!(
            neutralize_slack_mentions("see <https://x.test/a>"),
            "see <https://x.test/a>"
        );
        assert_eq!(
            neutralize_slack_mentions("`<!channel>`"),
            "`<!channel>`",
            "code already does not broadcast"
        );
    }

    #[test]
    fn a_body_that_fits_keeps_its_tail_untouched() {
        assert_eq!(
            truncate_with_tail("short", 100, Some("\n\ntail")),
            "short\n\ntail"
        );
        assert_eq!(truncate_with_tail("short", 100, None), "short");
    }

    #[test]
    fn an_oversize_body_is_cut_but_says_so_and_keeps_the_link() {
        let body = "a".repeat(200);
        let out = truncate_with_tail(&body, 80, Some("\n\nlink"));
        assert!(
            out.chars().count() <= 80,
            "got {} chars",
            out.chars().count()
        );
        assert!(out.contains("truncated by Maidan"));
        assert!(out.ends_with("\n\nlink"), "the backlink survives the cut");
        assert!(out.starts_with("aaa"));
    }

    #[test]
    fn a_cut_prefers_a_paragraph_break_near_the_end_of_the_budget() {
        let body = format!("{}\n\n{}", "a".repeat(70), "b".repeat(70));
        let out = truncate_with_tail(&body, 100, None);
        assert_eq!(
            out,
            format!("{}{TRUNCATION_NOTICE}", "a".repeat(70)),
            "the first paragraph is kept whole and the cut lands on the break"
        );
    }

    #[test]
    fn a_budget_too_small_for_any_content_still_returns_something_honest() {
        let out = truncate_with_tail(&"a".repeat(100), 20, Some("\n\nlink"));
        assert!(out.chars().count() <= 20);
        assert!(
            !out.contains("aaa"),
            "a fragment that claims to be a review is worse than none: {out:?}"
        );
    }

    #[test]
    fn truncation_cuts_on_a_character_boundary() {
        // Multi-byte throughout, so a byte-indexed cut would panic.
        let body = "é".repeat(500);
        let out = truncate_with_tail(&body, 120, Some("\n\nlink"));
        assert!(out.chars().count() <= 120);
        assert!(out.contains("truncated by Maidan"));
    }

    #[test]
    fn mrkdwn_rewrites_links_headings_and_bold() {
        assert_eq!(
            gfm_to_mrkdwn("see [the run](https://pi.test/r/1) now"),
            "see <https://pi.test/r/1|the run> now"
        );
        assert_eq!(gfm_to_mrkdwn("## Findings"), "*Findings*");
        assert_eq!(gfm_to_mrkdwn("### A **bold** head"), "*A *bold* head*");
        assert_eq!(
            gfm_to_mrkdwn("a **bold** and __also__ word"),
            "a *bold* and *also* word"
        );
    }

    #[test]
    fn mrkdwn_leaves_alone_what_it_does_not_understand() {
        // Single-delimiter emphasis and arithmetic: rewriting `*` would mangle these.
        assert_eq!(gfm_to_mrkdwn("2 * 3 and _it_"), "2 * 3 and _it_");
        // Code is preserved verbatim, GFM markers and all.
        assert_eq!(
            gfm_to_mrkdwn("`[x](y)` and ```\n**b**\n```"),
            "`[x](y)` and ```\n**b**\n```"
        );
        // An image is not a link, and a malformed link stays readable.
        assert_eq!(gfm_to_mrkdwn("![alt](u)"), "![alt](u)");
        assert_eq!(gfm_to_mrkdwn("[unclosed(u)"), "[unclosed(u)");
        assert_eq!(gfm_to_mrkdwn("[a](u v)"), "[a](u v)");
        // Not a heading: seven hashes, or hashes with no title.
        assert_eq!(gfm_to_mrkdwn("####### too deep"), "####### too deep");
        assert_eq!(gfm_to_mrkdwn("#"), "#");
    }

    #[test]
    fn the_github_body_defuses_mentions_and_appends_the_backlink() {
        let out = github_comment_body("review by @octocat", Some("https://pi.test/r/1"));
        assert!(out.starts_with("review by `@octocat`"));
        assert!(out.contains("[View in the producer](https://pi.test/r/1)"));
    }

    #[test]
    fn the_github_body_fits_githubs_ceiling_even_for_a_huge_rendered() {
        let out = github_comment_body(
            &"x".repeat(GITHUB_BODY_MAX_CHARS * 2),
            Some("https://p.test"),
        );
        assert!(out.chars().count() <= GITHUB_BODY_MAX_CHARS);
        assert!(out.contains("https://p.test"), "the link survives");
    }

    #[test]
    fn the_slack_body_is_the_summary_and_digest_in_mrkdwn_never_rendered() {
        let out = slack_message_body(
            "**3 findings** in [bgv3](https://x.test/pr/1)",
            &["- 1 critical".to_string(), "- 2 minor".to_string()],
            Some("https://pi.test/r/1"),
        );
        assert_eq!(
            out,
            "*3 findings* in <https://x.test/pr/1|bgv3>\n- 1 critical\n- 2 minor\n<https://pi.test/r/1|View in the producer>"
        );
    }

    #[test]
    fn the_slack_body_defuses_a_broadcast_smuggled_through_the_summary() {
        let out = slack_message_body("<!channel> ship it", &[], None);
        assert_eq!(out, "&lt;!channel> ship it");
    }

    #[test]
    fn segmenting_an_unclosed_fence_treats_the_rest_as_code() {
        // The conservative direction: better to leave text alone than to rewrite
        // something the renderer will show verbatim.
        assert_eq!(
            neutralize_github_mentions("before\n```\n@octocat"),
            "before\n```\n@octocat"
        );
        assert_eq!(
            neutralize_github_mentions("an `unclosed span @octocat"),
            "an `unclosed span @octocat"
        );
    }

    #[test]
    fn a_tilde_fence_is_code_too() {
        assert_eq!(
            neutralize_github_mentions("~~~\n@octocat\n~~~\nhi @a"),
            "~~~\n@octocat\n~~~\nhi `@a`"
        );
        // Inline strikethrough is not code, so prose rules still apply.
        assert_eq!(neutralize_github_mentions("~~gone @a~~"), "~~gone `@a`~~");
    }
}
