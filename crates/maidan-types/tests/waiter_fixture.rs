//! The contract lock (Cluster 379.2).
//!
//! These assertions run against the **authoritative** producer fixture, committed
//! at `tests/fixtures/pi_waiter_result_v1.json` and embedded here at compile time.
//! That is the whole point: if the producer changes the grammar Maidan routes on,
//! this test fails in this repo — loudly, in CI, before a delivery goes to the
//! wrong place in production. It is not a test of the parser so much as a
//! tripwire on somebody else's wire format.
//!
//! It deliberately asserts **only the fields Maidan routes on**. Pinning
//! `findings`, `cost_usd`, `per_seat` or `run_id` would make the lock fire on
//! changes that cannot affect delivery, and a tripwire that cries wolf gets
//! deleted.

use maidan_types::{
    parse_waiter_result, DeliverTarget, EgressTarget, WaiterResult, STATUS_REVIEWED,
    WAITER_RESULT_SCHEMA,
};

const FIXTURE: &str = include_str!("fixtures/pi_waiter_result_v1.json");

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
        value["result_kind"], "pi.review.result/1",
        "result_kind is a namespaced string and the search facet — not an enum"
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
        r.view_in_pi.as_deref(),
        Some("https://pi.beatgig.dev/runs/aa4dc966-0e09-44c3-b7a5-2d048b48b301")
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
    assert_eq!(r.pr.as_deref(), Some("beatgig/bgv3#3915"));
}

#[test]
fn the_authoritative_fixture_routes_to_both_surfaces() {
    let r = parsed();
    assert_eq!(
        r.deliver_to,
        vec![
            DeliverTarget::Github {
                repo: "beatgig/bgv3".into(),
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
/// between the producer's grammar (Cluster 379) and the transport (377/378), and
/// it is the one most likely to drift silently.
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
                repo: "beatgig/bgv3".into(),
                issue_number: 3915,
            },
            EgressTarget::Slack {
                channel_id: "C0123ABCDEF".into(),
            },
        ]
    );

    // The delivery grain is issue-qualified; the allowlist grain is the
    // repository. An operator blesses `github:beatgig/bgv3` once and it covers
    // this PR and every other one — per-issue blessing would be a ticket per PR.
    assert_eq!(projected[0].selector(), "beatgig/bgv3#3915");
    assert_eq!(projected[0].allowlist_selector(), "beatgig/bgv3");
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

/// Maidan carries the rest of the envelope through untouched and does not
/// interpret it. This asserts the *parser's* indifference, not the fields'
/// values, so the producer stays free to evolve them.
#[test]
fn fields_maidan_does_not_route_on_are_ignored_rather_than_required() {
    let mut value: serde_json::Value = serde_json::from_str(FIXTURE).expect("valid JSON");
    let before = parsed();

    let obj = value.as_object_mut().expect("an object");
    for uninterpreted in [
        "findings",
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
        "head_sha",
    ] {
        assert!(
            obj.remove(uninterpreted).is_some(),
            "{uninterpreted} is expected in the fixture; update this list if the producer drops it"
        );
    }

    let after = parse_waiter_result(&value).expect("still a recognized envelope");
    assert_eq!(
        after, before,
        "removing everything Maidan does not route on changes nothing it routes on"
    );
}
