//! Land-gate pointer (Cluster 385, renamed Cluster 389).
//!
//! A thread may hold a **land-gate pointer** — `{kind: "land_gate",
//! status: pass|fail, artifact_sha?}` plus the green/amber/red land
//! vocabulary. The room stores the pointer; an external verifier records
//! pass/fail. This is not a CI product and not a judge panel in the room.
//!
//! The FSM close-gate (Cluster 385.2) refuses `closed` unless a **qualifying
//! pass** exists: `status = pass`, `land = green`, recorded by a member who
//! has declared [`LAND_GATE_SKILL`], and that member is neither the
//! thread's owner nor its assignee (the implementer). **Amber** is
//! flags-then-still-engages — not a land. **Red** is a fail or an
//! unqualified pointer. No pointer and no requirement is additive (close
//! as before), matching Cluster 375's opt-in review gate.
//!
//! Cluster 383's critical→`request_changes` adapter is a separate
//! composition on the review gate; this module does not redo it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::MemberId;

/// The member-skill tag a gate-skilled verifier declares (Cluster 230
/// free-form skills). The close-gate only counts a pass from a member who
/// has this skill — an implementer who is not land-gate-skilled cannot
/// land their own work by writing a pointer.
pub const LAND_GATE_SKILL: &str = "land_gate";

/// Wire `kind` on the pointer. Always `"land_gate"`.
pub const LAND_GATE_KIND: &str = "land_gate";

/// Pass/fail recorded on the thread by a gate-skilled verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum LandGateStatus {
    Pass,
    Fail,
}

impl LandGateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pass" => Some(Self::Pass),
            "fail" => Some(Self::Fail),
            _ => None,
        }
    }
}

/// Land-gate vocabulary. Only [`LandColor::Green`] is a land.
///
/// * **green** — a qualifying LandGate pass; the FSM may `closed`.
/// * **amber** — flags-then-still-engages (accepted nonsense). Not a land.
/// * **red** — fail, pending requirement, or an unqualified pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum LandColor {
    Green,
    Amber,
    Red,
}

impl LandColor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Amber => "amber",
            Self::Red => "red",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "green" => Some(Self::Green),
            "amber" => Some(Self::Amber),
            "red" => Some(Self::Red),
            _ => None,
        }
    }
}

/// The pointer LandGate writes onto a thread. Small on purpose: kind,
/// pass/fail, optional artifact SHA, and the land color.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LandGatePointer {
    /// Always [`LAND_GATE_KIND`].
    pub kind: String,
    pub status: LandGateStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_sha: Option<String>,
    pub land: LandColor,
}

impl LandGatePointer {
    pub fn new(status: LandGateStatus, artifact_sha: Option<String>, land: LandColor) -> Self {
        Self {
            kind: LAND_GATE_KIND.to_string(),
            status,
            artifact_sha,
            land,
        }
    }
}

/// What the close-gate and `GET /threads/:id/land-gate` read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LandGateStanding {
    /// A row exists — the gate is armed (require and/or a recorded pointer).
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<LandGatePointer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_by: Option<MemberId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<DateTime<Utc>>,
    /// Computed land color (not merely what LandGate wrote). Green only
    /// when a qualifying pass exists, or when the gate is not armed.
    pub land: LandColor,
    /// `land == green`. The FSM close-gate requires this when `required`.
    pub landable: bool,
}

/// Stored pointer plus who wrote it. Internal to standing assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedLandGate {
    pub pointer: LandGatePointer,
    pub recorded_by: MemberId,
    pub recorded_at: DateTime<Utc>,
}

/// Resolve the stored land color from a `/test` status and an optional
/// requested color.
///
/// A **fail** is always red. A **pass** defaults to green; the caller may
/// request amber (flags-then-still-engages) or red (explicit refuse). A
/// requested green on a fail is ignored.
pub fn resolve_land(status: LandGateStatus, requested: Option<LandColor>) -> LandColor {
    match status {
        LandGateStatus::Fail => LandColor::Red,
        LandGateStatus::Pass => match requested {
            Some(LandColor::Amber) => LandColor::Amber,
            Some(LandColor::Red) => LandColor::Red,
            Some(LandColor::Green) | None => LandColor::Green,
        },
    }
}

/// A pass that may land: green, from a land-gate-skilled member who is
/// not the implementer (owner or assignee).
pub fn is_qualifying_pass(
    status: LandGateStatus,
    land: LandColor,
    recorded_by: MemberId,
    owner_id: Option<MemberId>,
    assignee_id: Option<MemberId>,
    recorder_has_skill: bool,
) -> bool {
    status == LandGateStatus::Pass
        && land == LandColor::Green
        && recorder_has_skill
        && owner_id != Some(recorded_by)
        && assignee_id != Some(recorded_by)
}

/// Gate-side land color. No row → green (additive). Armed with no pointer
/// → red (pending). A skilled third-party amber pass stays amber. Anything
/// else that is not a qualifying pass is red.
pub fn standing_land(
    pointer: Option<&LandGatePointer>,
    recorded_by: Option<MemberId>,
    owner_id: Option<MemberId>,
    assignee_id: Option<MemberId>,
    recorder_has_skill: bool,
    required: bool,
) -> LandColor {
    if !required {
        return LandColor::Green;
    }
    let Some(pointer) = pointer else {
        return LandColor::Red;
    };
    let Some(recorded_by) = recorded_by else {
        return LandColor::Red;
    };
    if is_qualifying_pass(
        pointer.status,
        pointer.land,
        recorded_by,
        owner_id,
        assignee_id,
        recorder_has_skill,
    ) {
        return LandColor::Green;
    }
    if pointer.status == LandGateStatus::Pass
        && pointer.land == LandColor::Amber
        && recorder_has_skill
        && owner_id != Some(recorded_by)
        && assignee_id != Some(recorded_by)
    {
        return LandColor::Amber;
    }
    LandColor::Red
}

/// Assemble standing from a stored row (or its absence) plus thread SoD
/// context.
pub fn land_gate_standing(
    required: bool,
    recorded: Option<RecordedLandGate>,
    owner_id: Option<MemberId>,
    assignee_id: Option<MemberId>,
    recorder_has_skill: bool,
) -> LandGateStanding {
    let pointer = recorded.as_ref().map(|r| r.pointer.clone());
    let recorded_by = recorded.as_ref().map(|r| r.recorded_by);
    let recorded_at = recorded.as_ref().map(|r| r.recorded_at);
    let land = standing_land(
        pointer.as_ref(),
        recorded_by,
        owner_id,
        assignee_id,
        recorder_has_skill,
        required,
    );
    LandGateStanding {
        required,
        pointer,
        recorded_by,
        recorded_at,
        land,
        landable: land == LandColor::Green,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn mid(n: u8) -> MemberId {
        MemberId(Uuid::from_u128(n as u128))
    }

    #[test]
    fn resolve_land_fail_is_always_red() {
        assert_eq!(resolve_land(LandGateStatus::Fail, None), LandColor::Red);
        assert_eq!(
            resolve_land(LandGateStatus::Fail, Some(LandColor::Green)),
            LandColor::Red
        );
        assert_eq!(
            resolve_land(LandGateStatus::Fail, Some(LandColor::Amber)),
            LandColor::Red
        );
    }

    #[test]
    fn resolve_land_pass_defaults_green_and_honors_amber() {
        assert_eq!(resolve_land(LandGateStatus::Pass, None), LandColor::Green);
        assert_eq!(
            resolve_land(LandGateStatus::Pass, Some(LandColor::Green)),
            LandColor::Green
        );
        assert_eq!(
            resolve_land(LandGateStatus::Pass, Some(LandColor::Amber)),
            LandColor::Amber
        );
        assert_eq!(
            resolve_land(LandGateStatus::Pass, Some(LandColor::Red)),
            LandColor::Red
        );
    }

    #[test]
    fn qualifying_pass_needs_skill_and_not_implementer() {
        let sc = mid(1);
        let owner = mid(2);
        let assignee = mid(3);
        assert!(is_qualifying_pass(
            LandGateStatus::Pass,
            LandColor::Green,
            sc,
            Some(owner),
            Some(assignee),
            true
        ));
        assert!(
            !is_qualifying_pass(
                LandGateStatus::Pass,
                LandColor::Green,
                sc,
                Some(owner),
                Some(assignee),
                false
            ),
            "unskilled"
        );
        assert!(
            !is_qualifying_pass(
                LandGateStatus::Pass,
                LandColor::Green,
                owner,
                Some(owner),
                Some(assignee),
                true
            ),
            "owner is the implementer"
        );
        assert!(
            !is_qualifying_pass(
                LandGateStatus::Pass,
                LandColor::Green,
                assignee,
                Some(owner),
                Some(assignee),
                true
            ),
            "assignee is the implementer"
        );
        assert!(
            !is_qualifying_pass(
                LandGateStatus::Pass,
                LandColor::Amber,
                sc,
                Some(owner),
                Some(assignee),
                true
            ),
            "amber is not a land"
        );
        assert!(!is_qualifying_pass(
            LandGateStatus::Fail,
            LandColor::Red,
            sc,
            Some(owner),
            Some(assignee),
            true
        ));
    }

    #[test]
    fn standing_land_vacuous_green_pending_red_amber_stays() {
        assert_eq!(
            standing_land(None, None, None, None, false, false),
            LandColor::Green
        );
        assert_eq!(
            standing_land(None, None, None, None, false, true),
            LandColor::Red
        );
        let sc = mid(1);
        let pointer = LandGatePointer::new(LandGateStatus::Pass, None, LandColor::Amber);
        assert_eq!(
            standing_land(Some(&pointer), Some(sc), None, None, true, true),
            LandColor::Amber
        );
        let green = LandGatePointer::new(LandGateStatus::Pass, None, LandColor::Green);
        assert_eq!(
            standing_land(Some(&green), Some(sc), None, None, true, true),
            LandColor::Green
        );
    }

    #[test]
    fn pointer_wire_shape_is_kind_status_optional_sha_and_land() {
        let p = LandGatePointer::new(LandGateStatus::Pass, Some("abc".into()), LandColor::Green);
        let v = serde_json::to_value(&p).expect("json");
        assert_eq!(v["kind"], LAND_GATE_KIND);
        assert_eq!(v["status"], "pass");
        assert_eq!(v["artifact_sha"], "abc");
        assert_eq!(v["land"], "green");
        let no_sha = LandGatePointer::new(LandGateStatus::Fail, None, LandColor::Red);
        let v = serde_json::to_value(&no_sha).expect("json");
        assert!(v.get("artifact_sha").is_none());
        assert_eq!(v["status"], "fail");
        assert_eq!(v["land"], "red");
    }
}
