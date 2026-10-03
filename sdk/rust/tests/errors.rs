//! The problem-type error variants and forward-compatible models (no server).

use maidan::{MaidanError, Thread, ThreadContext, ThreadState, PROBLEM_BASE, PROBLEM_TYPES};
use serde_json::json;

fn problem(type_: &str, status: u16) -> Vec<u8> {
    json!({ "type": type_, "title": "T", "status": status, "detail": "d" })
        .to_string()
        .into_bytes()
}

#[test]
fn every_problem_type_the_server_documents_has_its_own_variant() {
    let mut names = std::collections::BTreeSet::new();
    for segment in PROBLEM_TYPES {
        let err = MaidanError::from_response(
            418,
            &problem(&format!("{PROBLEM_BASE}{segment}"), 418),
            None,
        );
        assert!(
            !matches!(err, MaidanError::Unknown(_)),
            "{segment} fell back to Unknown"
        );
        let name = format!("{err:?}");
        let variant = name.split('(').next().unwrap().to_string();
        assert!(
            names.insert(variant.clone()),
            "{segment} shares the variant {variant}"
        );
    }
    assert_eq!(names.len(), 17);
}

#[test]
fn an_error_carries_status_type_title_detail_and_the_raw_problem() {
    let body = json!({
        "type": format!("{PROBLEM_BASE}not-found"),
        "title": "Not Found",
        "status": 404,
        "detail": "gone",
        "trace": "a member added later"
    });
    let err = MaidanError::from_response(404, body.to_string().as_bytes(), None);
    let MaidanError::NotFound(p) = &err else {
        panic!("expected NotFound, got {err:?}");
    };
    assert_eq!(p.status, 404);
    assert_eq!(p.title.as_deref(), Some("Not Found"));
    assert_eq!(p.detail.as_deref(), Some("gone"));
    assert_eq!(p.raw.as_ref(), Some(&body));
    assert_eq!(err.to_string(), "maidan: request failed: HTTP 404: gone");
}

#[test]
fn an_unknown_type_or_a_non_problem_body_is_unknown() {
    let err = MaidanError::from_response(
        409,
        &problem(&format!("{PROBLEM_BASE}added-next-year"), 409),
        None,
    );
    assert!(matches!(err, MaidanError::Unknown(_)));
    assert!(err.is_conflict());

    let err = MaidanError::from_response(502, b"<html>bad gateway</html>", None);
    let MaidanError::Unknown(p) = &err else {
        panic!("expected Unknown, got {err:?}");
    };
    assert_eq!(p.problem_type, None);
    assert_eq!(p.raw, None);
    assert_eq!(p.detail.as_deref(), Some("<html>bad gateway</html>"));
}

#[test]
fn retry_after_is_kept_on_an_overloaded_503() {
    let err = MaidanError::from_response(
        503,
        &problem(&format!("{PROBLEM_BASE}overloaded"), 503),
        Some(7.0),
    );
    assert!(matches!(err, MaidanError::Overloaded(_)));
    assert_eq!(err.retry_after(), Some(7.0));
}

#[test]
fn members_a_model_does_not_declare_are_kept_and_new_enum_values_decode() {
    let t: Thread = serde_json::from_value(json!({
        "id": "t1",
        "channel_id": "c1",
        "state": "blocked_on_mars",
        "created_at": "x",
        "updated_at": "x",
        "novel": { "deep": 1 }
    }))
    .unwrap();
    assert_eq!(t.id, "t1");
    assert_eq!(t.state, ThreadState::Other("blocked_on_mars".into()));
    assert_eq!(t.title, None);
    assert_eq!(t.extra["novel"], json!({ "deep": 1 }));
    assert_eq!(t.unknown_members(), ["Thread.novel"]);
}

#[test]
fn unknown_members_are_found_in_nested_models_and_missing_required_ones_fail() {
    let ts = "2026-09-29T00:00:00Z";
    let ctx: ThreadContext = serde_json::from_value(json!({
        "workspace_id": "w",
        "channel_id": "c1",
        "thread_id": "t1",
        "thread": { "created_at": ts },
        "messages": [{ "id": "m", "thread_id": "t1", "author_id": "a", "body": "b", "posted_at": ts, "later": 1 }],
        "message_edits": [],
        "references": [],
        "artifacts": [],
        "transitions": [],
        "state": "open",
        "updated_at": ts,
        "prefix_sha256": "ab",
        "prefix_bytes": 1
    }))
    .unwrap();
    assert!(ctx.glossary.is_empty());
    assert_eq!(ctx.unknown_members(), ["ThreadContext.messages[0].later"]);
    assert!(serde_json::from_value::<Thread>(json!({ "id": "t1" })).is_err());
}
