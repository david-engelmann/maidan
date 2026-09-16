//! Cluster 397.5: what leaves Maidan for an external surface cannot ping real
//! humans from bytes an agent chose.
//!
//! Each test here is a concrete escape that worked before. The defusal itself
//! shipped in 378.3 — these are the four ways around it.

use maidan_server::egress_body::{
    github_comment_body, neutralize_github_mentions, neutralize_slack_mentions, truncate_with_tail,
};

/// Slack's `<!channel>` is not Markdown — it is Slack's own escape sequence,
/// expanded when the message is parsed. Backticks do not stop it notifying, so
/// skipping code spans (correct for GitHub) was a live broadcast here.
#[test]
fn slack_escaping_applies_inside_code_spans_and_fences() {
    for input in [
        "`<!channel>` ping",
        "see `<@U0123ABC>` here",
        "```\n<!here>\n```",
        "` <!channel> unclosed backtick",
    ] {
        let out = neutralize_slack_mentions(input);
        assert!(
            !out.contains("<!") && !out.contains("<@"),
            "a live Slack mention survived: {input:?} -> {out:?}"
        );
        assert!(out.contains("&lt;"), "it should still be readable: {out:?}");
    }
}

/// A plain `<https://…>` autolink is not a mention and must survive.
#[test]
fn slack_escaping_leaves_autolinks_alone() {
    let out = neutralize_slack_mentions("see <https://example.test/x> for more");
    assert_eq!(out, "see <https://example.test/x> for more");
}

/// One unmatched backtick used to classify the entire rest of the input as
/// code, switching off GitHub mention defusal for everything after it. A stray
/// backtick is trivially plantable in a quoted diff.
#[test]
fn an_unmatched_backtick_does_not_disable_github_defusal() {
    let out = neutralize_github_mentions("a stray ` tick then @octocat please look");
    assert!(
        out.contains("`@octocat`"),
        "the mention after an unmatched backtick must still be defused: {out:?}"
    );

    // A real code span is still respected — a mention inside it does not notify
    // on GitHub, and rewriting it would corrupt what the reader is reading.
    let spanned = neutralize_github_mentions("in code `@octocat` stays verbatim");
    assert_eq!(spanned, "in code `@octocat` stays verbatim");

    // An unclosed *fence* genuinely does run to end of input.
    let fenced = neutralize_github_mentions("```\n@octocat\n");
    assert_eq!(fenced, "```\n@octocat\n", "an unclosed fence is still code");
}

/// The backlink is appended after defusal, and `[text](…)` ends at the first
/// `)` — so a crafted `view_url` closed the link and everything after it
/// rendered as live Markdown, mention included.
#[test]
fn a_crafted_backlink_cannot_break_out_of_the_markdown_link() {
    let out = github_comment_body("clean review", Some("https://x.test) @acme/platform ("));
    assert!(
        !out.contains("@acme/platform") || out.contains("`@acme/platform`"),
        "a backlink must not smuggle a live mention: {out:?}"
    );
    assert!(
        !out.contains("View in the producer"),
        "an unusable backlink is dropped rather than escaped into the page: {out:?}"
    );

    // An ordinary URL still gets its backlink.
    let ok = github_comment_body("clean review", Some("https://x.test/runs/1"));
    assert!(ok.contains("[View in the producer](https://x.test/runs/1)"));

    // A non-http scheme is refused.
    let js = github_comment_body("clean", Some("javascript:alert(1)"));
    assert!(!js.contains("View in the producer"), "{js:?}");
}

/// Defusal wraps `@x` as `` `@x` ``. A cut landing between the two backticks
/// left the opener dangling — and an unmatched backtick renders literally, so
/// the mention came back live. The producer controls both the length and the
/// position, so hitting the boundary is deterministic, not luck.
#[test]
fn truncation_never_splits_a_defusing_code_span() {
    for pad in 0..80usize {
        let body = format!(
            "{}@octocat and a good deal more text after it",
            "x".repeat(pad)
        );
        let safe = neutralize_github_mentions(&body);
        let out = truncate_with_tail(&safe, 72, None);
        let ticks = out.chars().filter(|c| *c == '`').count();
        assert_eq!(
            ticks % 2,
            0,
            "pad={pad} left an unbalanced backtick, re-exposing the mention: {out:?}"
        );
    }
}

/// The whole chain, on the path that actually shipped the bug: a non-`reviewed`
/// result interpolates the producer's `status` into a Maidan-authored notice,
/// and the Slack branch returned that notice raw — no escaping, no ceiling. Any
/// caller who can write a thread result could broadcast to a blessed channel.
#[test]
fn a_hostile_status_cannot_broadcast_through_the_failure_notice() {
    use maidan_server::result_delivery::delivery_body;
    use maidan_types::{parse_waiter_result, EgressTarget, ThreadId, WAITER_RESULT_SCHEMA};

    let envelope = serde_json::json!({
        "schema": WAITER_RESULT_SCHEMA,
        "result_kind": "example.review.result/1",
        "status": "<!channel> <!here> <@U0123ABC> everybody look",
        "deliver_to": [{"surface": "slack", "channel": "C0123ABCDEF"}],
    });
    let waiter = parse_waiter_result(&envelope).expect("envelope parses");
    assert!(!waiter.is_reviewed(), "this is the failure path");

    let body = delivery_body(
        ThreadId::new(),
        &EgressTarget::Slack {
            channel_id: "C0123ABCDEF".into(),
        },
        &waiter,
    );
    assert!(
        !body.contains("<!") && !body.contains("<@"),
        "a hostile status reached Slack live: {body:?}"
    );
    assert!(
        body.contains("&lt;!channel&gt;") || body.contains("&lt;!channel>"),
        "the reader should still see what was said: {body:?}"
    );

    // GitHub's own path is unchanged and still defused.
    let gh = delivery_body(
        ThreadId::new(),
        &EgressTarget::Github {
            repo: "example/repo".into(),
            issue_number: 7,
        },
        &waiter,
    );
    assert!(
        gh.contains("maidan:result:"),
        "marker still at byte 0: {gh:?}"
    );
}
