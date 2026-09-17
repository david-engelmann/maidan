//! Which member skills are **governance-bearing**.
//!
//! Skills are free-form tags an agent declares, and routing them is set
//! containment — `claim_next` hands a task to anyone who declares what it
//! requires. That openness is the point, and for routing it is harmless: the
//! worst a bogus skill buys you is work you cannot do.
//!
//! Two of them are not routing tags. The close-gate only counts a green pass
//! from a member who declared [`LAND_GATE_SKILL`], and the adapter only arms
//! `request_changes` for a producer who declared [`REVIEW_SKILL`]. For those
//! two, *declaring the skill is what qualifies you to approve* — so a
//! self-service grant hands the holder the qualification the gate exists to
//! check, and the separation-of-duties test is all that is left standing.
//!
//! So granting one ratchets, exactly as the gates themselves ratchet:
//! the ordinary path keeps `workspace:write`, and the operation that *widens*
//! who may approve needs `channel:admin` — which lives in `maidan.human.admin`
//! and deliberately not in `maidan.agent.worker`.
//!
//! **Removing** one is not gated. It narrows the set of qualified approvers, so
//! at worst it blocks a thread from closing; it can never let something through
//! the gate. Gating the tightening direction would only make governance harder
//! to maintain without making it harder to subvert.

use crate::land_gate::LAND_GATE_SKILL;
use crate::review::REVIEW_SKILL;

/// The skills whose *presence on a member* is read as authority by a gate.
///
/// Kept sorted so [`is_governance_skill`] can binary-search, and asserted sorted
/// by a test — an out-of-order entry would silently stop gating a skill.
pub const GOVERNANCE_SKILLS: &[&str] = &[LAND_GATE_SKILL, REVIEW_SKILL];

/// True when granting `skill` widens who may approve, and so needs the
/// elevated capability.
///
/// Compared case-insensitively against the trimmed value, because the grant
/// surfaces store what the caller sent: `" Land_Gate "` must not walk past a
/// gate that `land_gate` would not.
pub fn is_governance_skill(skill: &str) -> bool {
    let normalized = skill.trim().to_ascii_lowercase();
    GOVERNANCE_SKILLS
        .binary_search(&normalized.as_str())
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_governance_set_is_sorted_for_binary_search() {
        let mut sorted = GOVERNANCE_SKILLS.to_vec();
        sorted.sort_unstable();
        assert_eq!(GOVERNANCE_SKILLS, sorted.as_slice());
    }

    #[test]
    fn governance_skills_are_recognized_however_they_are_spelled() {
        for spelling in [
            "land_gate",
            "  land_gate  ",
            "LAND_GATE",
            "Land_Gate",
            "review",
            "REVIEW",
        ] {
            assert!(
                is_governance_skill(spelling),
                "{spelling} must be recognized as governance-bearing"
            );
        }
    }

    /// Ordinary routing tags stay self-service — the gate is narrow on purpose.
    #[test]
    fn routing_skills_are_not_governance() {
        for skill in ["rust", "land_gates", "reviewer", "pre-review", "", "gate"] {
            assert!(!is_governance_skill(skill), "{skill} must stay open");
        }
    }
}
