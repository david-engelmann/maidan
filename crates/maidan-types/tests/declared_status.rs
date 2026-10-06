//! DeclaredStatus: the agent's self-reported status. `stalled` is
//! system-computed only and can never be declared.

use maidan_types::DeclaredStatus;

#[test]
fn all_variants_round_trip() {
    for status in DeclaredStatus::ALL {
        let s = status.as_str();
        let parsed = DeclaredStatus::parse(s).expect("parse");
        assert_eq!(&parsed, status, "round-trip for {s}");
    }
    // Exhaustive: if a variant is added, ALL must be updated and this
    // test will fail until the match below is extended.
    for status in DeclaredStatus::ALL {
        match status {
            DeclaredStatus::Working
            | DeclaredStatus::NeedsInput
            | DeclaredStatus::NeedsReview
            | DeclaredStatus::Blocked
            | DeclaredStatus::Done => {}
        }
    }
}

#[test]
fn stalled_is_refused() {
    // `stalled` is system-computed only; parse must refuse it.
    assert_eq!(DeclaredStatus::parse("stalled"), None);
    assert_eq!(DeclaredStatus::parse("STALLED"), None);
    assert_eq!(DeclaredStatus::parse(""), None);
    assert_eq!(DeclaredStatus::parse("unknown"), None);
}

#[test]
fn as_str_values() {
    assert_eq!(DeclaredStatus::Working.as_str(), "working");
    assert_eq!(DeclaredStatus::NeedsInput.as_str(), "needs_input");
    assert_eq!(DeclaredStatus::NeedsReview.as_str(), "needs_review");
    assert_eq!(DeclaredStatus::Blocked.as_str(), "blocked");
    assert_eq!(DeclaredStatus::Done.as_str(), "done");
}
