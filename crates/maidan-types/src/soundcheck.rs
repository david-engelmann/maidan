//! Soundcheck gate pointer (Cluster 384, Wave 2 #25 remainder, G-dev-6).
//!
//! A thread may hold a **Soundcheck pointer** — `{kind: "soundcheck",
//! status: pass|fail, artifact_sha?}` plus the green/amber/red land
//! vocabulary. The room stores the pointer; Soundcheck owns `/test`. This
//! is not a CI product and not a judge panel in the room.
//!
//! The FSM close-gate (Cluster 384.2) refuses `closed` unless a **qualifying
//! pass** exists: `status = pass`, `land = green`, recorded by a member who
//! has declared [`SOUNDCHECK_SKILL`], and that member is neither the
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

/// The member-skill tag a Soundcheck agent declares (Cluster 230 free-form
/// skills). The close-gate only counts a pass from a member who has this
/// skill — an implementer who is not soundcheck-skilled cannot land their
/// own work by writing a pointer.
pub const SOUNDCHECK_SKILL: &str = "soundcheck";

/// Wire `kind` on the pointer. Always `"soundcheck"`.
pub const SOUNDCHECK_KIND: &str = "soundcheck";

/// Soundcheck `/test` result stored on the thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum SoundcheckStatus {
    Pass,
    Fail,
}

impl SoundcheckStatus {
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
/// * **green** — a qualifying Soundcheck pass; the FSM may `closed`.
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

/// The pointer Soundcheck writes onto a thread. Small on purpose: kind,
/// pass/fail, optional artifact SHA, and the land color.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoundcheckPointer {
    /// Always [`SOUNDCHECK_KIND`].
    pub kind: String,
    pub status: SoundcheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_sha: Option<String>,
    pub land: LandColor,
}

impl SoundcheckPointer {
    pub fn new(status: SoundcheckStatus, artifact_sha: Option<String>, land: LandColor) -> Self {
        Self {
            kind: SOUNDCHECK_KIND.to_string(),
            status,
            artifact_sha,
            land,
        }
    }
}

/// What the close-gate and `GET /threads/:id/soundcheck` read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoundcheckStanding {
    /// A row exists — the gate is armed (require and/or a recorded pointer).
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<SoundcheckPointer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_by: Option<MemberId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at: Option<DateTime<Utc>>,
    /// Computed land color (not merely what Soundcheck wrote). Green only
    /// when a qualifying pass exists, or when the gate is not armed.
    pub land: LandColor,
    /// `land == green`. The FSM close-gate requires this when `required`.
    pub landable: bool,
}

/// Stored pointer plus who wrote it. Internal to standing assembly.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedSoundcheck {
    pub pointer: SoundcheckPointer,
    pub recorded_by: MemberId,
    pub recorded_at: DateTime<Utc>,
}

/// Resolve the stored land color from a `/test` status and an optional
/// requested color.
///
/// A **fail** is always red. A **pass** defaults to green; the caller may
/// request amber (flags-then-still-engages) or red (explicit refuse). A
/// requested green on a fail is ignored.
pub fn resolve_land(status: SoundcheckStatus, requested: Option<LandColor>) -> LandColor {
    match status {
        SoundcheckStatus::Fail => LandColor::Red,
        SoundcheckStatus::Pass => match requested {
            Some(LandColor::Amber) => LandColor::Amber,
            Some(LandColor::Red) => LandColor::Red,
            Some(LandColor::Green) | None => LandColor::Green,
        },
    }
}

/// A pass that may land: green, from a soundcheck-skilled member who is
/// not the implementer (owner or assignee).
pub fn is_qualifying_pass(
    status: SoundcheckStatus,
    land: LandColor,
    recorded_by: MemberId,
    owner_id: Option<MemberId>,
    assignee_id: Option<MemberId>,
    recorder_has_skill: bool,
) -> bool {
    status == SoundcheckStatus::Pass
        && land == LandColor::Green
        && recorder_has_skill
        && owner_id != Some(recorded_by)
        && assignee_id != Some(recorded_by)
}

/// Gate-side land color. No row → green (additive). Armed with no pointer
/// → red (pending). A skilled third-party amber pass stays amber. Anything
/// else that is not a qualifying pass is red.
pub fn standing_land(
    pointer: Option<&SoundcheckPointer>,
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
    if pointer.status == SoundcheckStatus::Pass
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
pub fn soundcheck_standing(
    required: bool,
    recorded: Option<RecordedSoundcheck>,
    owner_id: Option<MemberId>,
    assignee_id: Option<MemberId>,
    recorder_has_skill: bool,
) -> SoundcheckStanding {
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
    SoundcheckStanding {
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
        assert_eq!(resolve_land(SoundcheckStatus::Fail, None), LandColor::Red);
        assert_eq!(
            resolve_land(SoundcheckStatus::Fail, Some(LandColor::Green)),
            LandColor::Red
        );
        assert_eq!(
            resolve_land(SoundcheckStatus::Fail, Some(LandColor::Amber)),
            LandColor::Red
        );
    }

    #[test]
    fn resolve_land_pass_defaults_green_and_honors_amber() {
        assert_eq!(resolve_land(SoundcheckStatus::Pass, None), LandColor::Green);
        assert_eq!(
            resolve_land(SoundcheckStatus::Pass, Some(LandColor::Green)),
            LandColor::Green
        );
        assert_eq!(
            resolve_land(SoundcheckStatus::Pass, Some(LandColor::Amber)),
            LandColor::Amber
        );
        assert_eq!(
            resolve_land(SoundcheckStatus::Pass, Some(LandColor::Red)),
            LandColor::Red
        );
    }

    #[test]
    fn qualifying_pass_needs_skill_and_not_implementer() {
        let sc = mid(1);
        let owner = mid(2);
        let assignee = mid(3);
        assert!(is_qualifying_pass(
            SoundcheckStatus::Pass,
            LandColor::Green,
            sc,
            Some(owner),
            Some(assignee),
            true
        ));
        assert!(
            !is_qualifying_pass(
                SoundcheckStatus::Pass,
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
                SoundcheckStatus::Pass,
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
                SoundcheckStatus::Pass,
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
                SoundcheckStatus::Pass,
                LandColor::Amber,
                sc,
                Some(owner),
                Some(assignee),
                true
            ),
            "amber is not a land"
        );
        assert!(!is_qualifying_pass(
            SoundcheckStatus::Fail,
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
        let pointer = SoundcheckPointer::new(SoundcheckStatus::Pass, None, LandColor::Amber);
        assert_eq!(
            standing_land(Some(&pointer), Some(sc), None, None, true, true),
            LandColor::Amber
        );
        let green = SoundcheckPointer::new(SoundcheckStatus::Pass, None, LandColor::Green);
        assert_eq!(
            standing_land(Some(&green), Some(sc), None, None, true, true),
            LandColor::Green
        );
    }

    #[test]
    fn pointer_wire_shape_is_kind_status_optional_sha_and_land() {
        let p =
            SoundcheckPointer::new(SoundcheckStatus::Pass, Some("abc".into()), LandColor::Green);
        let v = serde_json::to_value(&p).expect("json");
        assert_eq!(v["kind"], SOUNDCHECK_KIND);
        assert_eq!(v["status"], "pass");
        assert_eq!(v["artifact_sha"], "abc");
        assert_eq!(v["land"], "green");
        let no_sha = SoundcheckPointer::new(SoundcheckStatus::Fail, None, LandColor::Red);
        let v = serde_json::to_value(&no_sha).expect("json");
        assert!(v.get("artifact_sha").is_none());
        assert_eq!(v["status"], "fail");
        assert_eq!(v["land"], "red");
    }
}
