//! The contract lock.
//!
//! These assertions run against the **authoritative** producer fixture,
//! committed at `tests/fixtures/waiter_result_v1.json` and embedded here at
//! compile time. That is the whole point: if the producer changes the grammar
//! Maidan routes on, this test fails in this repo — loudly, in CI, before a
//! delivery goes to the wrong place in production. It is not a test of the
//! parser so much as a tripwire on somebody else's wire format.
//!
//! It asserts the fields Maidan routes on **and** the inline comment pin
//! (`head_sha`, `findings[].line_range`). Pinning `corroboration`,
//! `cost_usd` or `per_seat` would make the lock fire on changes that cannot
//! affect delivery, and a tripwire that cries wolf gets deleted.
//!
//! `run_id` is still **not** a delivery-routing field — `parse_waiter_result`
//! ignores it. It is lineage, read via [`run_id_from_payload`] (a
//! separate extractor); see
//! `the_authoritative_fixture_run_id_is_accepted_as_parent_run_id`.

use maidan_types::{
    parse_waiter_result, result_kind_from_payload, review_decision_from_waiter,
    run_id_from_payload, DeliverTarget, EgressTarget, FindingLineRange, GithubDiffSide,
    ReviewDecision, WaiterResult, EXAMPLE_REVIEW_RESULT_KIND, FINDING_SEVERITY_CRITICAL,
    STATUS_REVIEWED, WAITER_RESULT_SCHEMA,
};

const FIXTURE: &str = include_str!("fixtures/waiter_result_v1.json");

fn parsed() -> WaiterResult {
    let value: serde_json::Value =
        serde_json::from_str(FIXTURE).expect("the committed fixture is valid JSON");
    parse_waiter_result(&value).expect("the committed fixture is a recognized envelope")
}

#[test]
fn the_authoritative_fixture_still_declares_the_schema_we_route_on() {
    let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid JSON");
    assert_eq!(
        value["schema"], WAITER_RESULT_SCHEMA,
        "the producer changed the envelope discriminator; delivery would go inert"
    );
    assert_eq!(
        value["result_kind"], EXAMPLE_REVIEW_RESULT_KIND,
        "result_kind is a namespaced string and the search facet — not an enum"
    );
    assert_eq!(
        result_kind_from_payload(&value),
        Some("example.review.result/1"),
        "the search-facet extractor reads the same namespaced string the lock pins"
    );
}

#[test]
fn the_authoritative_fixture_is_deliverable_and_carries_its_bodies() {
    let r = parsed();
    assert_eq!(r.status, STATUS_REVIEWED);
    assert!(
        r.is_reviewed(),
        "only a reviewed result delivers the producer's own bytes"
    );

    // `rendered` is the GitHub body and is large — a 2 KB review is the normal
    // case, which is why the egress body rules truncate rather than assume.
    let rendered = r
        .rendered
        .as_deref()
        .expect("a reviewed result has rendered");
    assert!(
        rendered.len() > 1000,
        "rendered is {} bytes; the fixture is supposed to be a realistic review",
        rendered.len()
    );
    assert!(
        rendered.contains("```python"),
        "rendered quotes code, which is exactly why mentions are neutralized at egress"
    );

    assert_eq!(
        r.summary.as_deref(),
        Some("Code review: 2 finding(s) (1 corroborated) across 2/2 seat(s)"),
        "summary is the Slack body — Slack never receives rendered"
    );
    assert_eq!(
        r.view_url.as_deref(),
        Some("https://producer.example.test/runs/aa4dc966-0e09-44c3-b7a5-2d048b48b301")
    );
}

/// The top-level `pr` is a **string** back-reference, while `deliver_to[].pr` is
/// an **integer** issue number. Two fields, same name, different types — this
/// assertion exists because reading the spec alone would not tell you that, and
/// getting it backwards is a deserialization error that only shows up on a real
/// payload.
#[test]
fn the_two_pr_fields_keep_their_different_types() {
    let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid JSON");
    assert!(
        value["pr"].is_string(),
        "top-level pr is a human back-reference string"
    );
    assert!(
        value["deliver_to"][0]["pr"].is_i64(),
        "a github target's pr is an issue number"
    );

    let r = parsed();
    assert_eq!(r.pr.as_deref(), Some("example/repo#42"));
}

#[test]
fn the_authoritative_fixture_routes_to_both_surfaces() {
    let r = parsed();
    assert_eq!(
        r.deliver_to,
        vec![
            DeliverTarget::Github {
                repo: "example/repo".into(),
                pr: 3915,
            },
            DeliverTarget::Slack {
                channel: "C0123ABCDEF".into(),
            },
        ],
        "the producer's routing intent, parsed exactly"
    );
}

/// Every target the fixture names has to survive the projection onto the egress
/// queue's vocabulary, or delivery stops at the type boundary. This is the seam
/// between the producer's grammar and the transport (377/378), and it is the
/// one most likely to drift silently.
#[test]
fn every_fixture_target_projects_onto_a_deliverable_egress_target() {
    let r = parsed();
    let projected: Vec<EgressTarget> = r
        .deliver_to
        .iter()
        .map(|t| {
            t.to_egress_target()
                .unwrap_or_else(|| panic!("{t:?} does not project onto an egress target"))
        })
        .collect();

    assert_eq!(
        projected,
        vec![
            EgressTarget::Github {
                repo: "example/repo".into(),
                issue_number: 3915,
            },
            EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
            },
        ]
    );

    // The delivery grain is issue-qualified; the allowlist grain is the
    // repository. An operator blesses `github:example/repo` once and it covers
    // this PR and every other one — per-issue blessing would be a ticket per PR.
    assert_eq!(projected[0].selector(), "example/repo#3915");
    assert_eq!(projected[0].allowlist_selector(), "example/repo");
    // Slack has no such split: a channel id is already what an operator blesses.
    assert_eq!(projected[1].selector(), projected[1].allowlist_selector());
}

/// The Slack channel must be an **id**, not a `#name`. A name is mutable, so the
/// channel it points at can change under both the delivery and the operator's
/// blessing — and an allowlist keyed on one is not an allowlist.
#[test]
fn the_fixtures_slack_channel_is_an_id_not_a_name() {
    let r = parsed();
    let DeliverTarget::Slack { channel } = &r.deliver_to[1] else {
        panic!("expected the second target to be slack");
    };
    assert!(
        channel.starts_with('C') || channel.starts_with('G'),
        "slack channel {channel:?} must be an id"
    );
    assert!(!channel.starts_with('#'), "a #name is not addressable");
}

/// `head_sha` is GitHub `commit_id`, and `line_range` is file-absolute
/// post-image / RIGHT, 1-indexed inclusive. A producer-side change to either
/// breaks this lock before 380.2 posts a comment on the wrong line of the wrong
/// commit.
#[test]
fn the_authoritative_fixture_pins_head_sha_and_post_image_findings() {
    let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid JSON");
    assert_eq!(
        value["head_sha"], "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911",
        "the producer dropped or renamed head_sha; inline comments would have no commit_id"
    );
    assert!(
        value["findings"].as_array().is_some_and(|f| !f.is_empty()),
        "the fixture is supposed to carry findings for the inline-comment path"
    );

    let r = parsed();
    assert_eq!(
        r.review_commit_id(),
        Some("b5e54f94fd04d6ef7d6e1197ddd59ace70edb911"),
        "commit_id is the envelope sha — never a live PR head"
    );
    assert_eq!(
        r.findings.len(),
        2,
        "both fixture findings have file + body + a valid line_range"
    );
    assert_eq!(r.findings[0].file, "auth.py");
    assert_eq!(
        r.findings[0].line_range,
        FindingLineRange { start: 2, end: 4 }
    );
    assert!(
        !r.findings[0].body.is_empty(),
        "the comment body is the producer's finding body"
    );

    let comments = r.github_review_comments();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0].path, "auth.py");
    assert_eq!(
        comments[0].line, 4,
        "GitHub line is the last line of the range"
    );
    assert_eq!(comments[0].start_line, Some(2));
    assert_eq!(comments[0].side, GithubDiffSide::Right);
    assert_eq!(comments[0].side.as_str(), "RIGHT");
    assert_eq!(comments[1].line, 4);
    assert_eq!(comments[1].start_line, Some(1));
}

/// The fixture's first finding is `critical`, so the producer→reviewer adapter
/// maps it to a `request_changes` decision. Severity stays a free string (the
/// second finding is `warning`); we do not close an enum of severities any more
/// than we close `result_kind`.
#[test]
fn the_authoritative_fixture_is_a_critical_request_changes() {
    let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid JSON");
    assert_eq!(
        value["findings"][0]["severity"], FINDING_SEVERITY_CRITICAL,
        "the fixture is supposed to carry a critical finding for the close-gate adapter"
    );
    assert_eq!(value["findings"][1]["severity"], "warning");

    let r = parsed();
    assert_eq!(
        r.findings[0].severity.as_deref(),
        Some(FINDING_SEVERITY_CRITICAL)
    );
    assert_eq!(r.findings[1].severity.as_deref(), Some("warning"));
    assert_eq!(
        review_decision_from_waiter(&value),
        Some(ReviewDecision::RequestChanges),
        "a delivered example.review.result/1 with any critical finding is request_changes"
    );
}

/// Maidan carries the rest of the envelope through untouched and does not
/// interpret it. This asserts the *parser's* indifference, not the fields'
/// values, so the producer stays free to evolve them. `head_sha` and `findings`
/// are inline-comment routing fields — removing them still parses the summary
/// path, but those fields go empty.
#[test]
fn fields_maidan_does_not_route_on_are_ignored_rather_than_required() {
    let mut value: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid JSON");
    let before = parsed();

    {
        let obj = value.as_object_mut().expect("an object");
        for uninterpreted in [
            "corroboration",
            "per_seat",
            "seats",
            "seats_reviewed",
            "run_id",
            "cost_usd",
            "duration_secs",
            "sandbox",
            "finding_count",
            "diff_available",
        ] {
            assert!(
                obj.remove(uninterpreted).is_some(),
                "{uninterpreted} is expected in the fixture; update this list if the producer drops it"
            );
        }
    }

    let after = parse_waiter_result(&value).expect("still a recognized envelope");
    assert_eq!(
        after, before,
        "removing everything Maidan does not route on changes nothing it routes on"
    );

    {
        let obj = value.as_object_mut().expect("an object");
        obj.remove("head_sha");
        obj.remove("findings");
    }

    let summary_only = parse_waiter_result(&value).expect("379 summary path still parses");
    assert_eq!(summary_only.review_commit_id(), None);
    assert!(summary_only.findings.is_empty());
    assert_eq!(summary_only.deliver_to, before.deliver_to);
    assert_eq!(summary_only.rendered, before.rendered);
}

/// The fixture's `run_id` is the first real producer value. Lineage accepts
/// that string as `parent_run_id` — it does not mint a parallel id. Delivery
/// still ignores it (`parse_waiter_result`).
#[test]
fn the_authoritative_fixture_run_id_is_accepted_as_parent_run_id() {
    let value: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid JSON");
    assert_eq!(
        run_id_from_payload(&value),
        Some("aa4dc966-0e09-44c3-b7a5-2d048b48b301"),
        "the producer run_id is the lineage value"
    );
    assert!(
        parse_waiter_result(&value).is_some(),
        "homing lineage does not change the delivery parse"
    );
}
