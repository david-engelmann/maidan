//! Named capability sets + Levy/Madden attenuation.
//!
//! A named set (`maidan.agent.worker`, `maidan.human.admin`) is a **mint-time
//! recipe**, not a stored capability string. Tokens still hold atomic
//! `workspace:read` / `message:post` / … rights. Expanding a set and then
//! dropping rights is progressive grant; the holder of a token may derive a
//! weaker one without `token:admin` (holder-side attenuation).
//!
//! This is object-capability attenuation (Levy/Madden), **not** a Cedar policy
//! rewrite. You can only drop rights you already hold. You cannot amplify.
//! Federation peer caps stay out of both named sets — those are minted on peer
//! tokens, not member tokens.

use chrono::{DateTime, Utc};

use crate::capability::{
    self, ARTIFACT_UPLOAD, AUDIT_READ_GLOBAL, CHANNEL_ADMIN, EVENT_SUBSCRIBE, MESSAGE_POST,
    OPERATOR_GLOBAL, SEARCH_QUERY, SECRET_ADMIN, SECRET_READ, THREAD_TRANSITION, TOKEN_ADMIN,
    WORKSPACE_READ, WORKSPACE_WRITE,
};

/// Worker agent bundle — collaborate, not administer.
pub const AGENT_WORKER: &str = "maidan.agent.worker";
/// Human operator bundle — member work plus token / channel / secret / audit.
pub const HUMAN_ADMIN: &str = "maidan.human.admin";

/// A named set and the atomic capabilities it expands to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySet {
    pub name: String,
    pub capabilities: Vec<String>,
}

fn worker_caps() -> Vec<String> {
    vec![
        WORKSPACE_READ.into(),
        WORKSPACE_WRITE.into(),
        MESSAGE_POST.into(),
        EVENT_SUBSCRIBE.into(),
        SEARCH_QUERY.into(),
        ARTIFACT_UPLOAD.into(),
        THREAD_TRANSITION.into(),
    ]
}

fn admin_caps() -> Vec<String> {
    let mut caps = worker_caps();
    caps.extend([
        TOKEN_ADMIN.into(),
        CHANNEL_ADMIN.into(),
        SECRET_READ.into(),
        SECRET_ADMIN.into(),
        AUDIT_READ_GLOBAL.into(),
        OPERATOR_GLOBAL.into(),
    ]);
    caps
}

/// Catalog of named sets. Order is the public contract.
pub fn named_sets() -> Vec<CapabilitySet> {
    vec![
        CapabilitySet {
            name: AGENT_WORKER.into(),
            capabilities: worker_caps(),
        },
        CapabilitySet {
            name: HUMAN_ADMIN.into(),
            capabilities: admin_caps(),
        },
    ]
}

pub fn is_named_set(name: &str) -> bool {
    name == AGENT_WORKER || name == HUMAN_ADMIN
}

/// Expand a named set. Unknown names fail closed (they are not capabilities).
pub fn expand_set(name: &str) -> Result<Vec<String>, String> {
    named_sets()
        .into_iter()
        .find(|s| s.name == name)
        .map(|s| s.capabilities)
        .ok_or_else(|| format!("unknown capability set: {name}"))
}

/// Named sets whose expansion is ⊆ `held`.
pub fn held_sets(held: &[String]) -> Vec<String> {
    named_sets()
        .into_iter()
        .filter(|s| s.capabilities.iter().all(|c| held.iter().any(|h| h == c)))
        .map(|s| s.name)
        .collect()
}

fn validate_held(held: &[String], requested: &[String]) -> Result<(), String> {
    capability::validate_list(requested)?;
    for cap in requested {
        if !held.iter().any(|h| h == cap) {
            return Err(format!("capability {cap} exceeds holder grant"));
        }
    }
    Ok(())
}

/// Dedup while preserving first-seen order (holder / set order).
fn unique_in_order(caps: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(caps.len());
    for cap in caps {
        if !out.iter().any(|c| c == cap) {
            out.push(cap.clone());
        }
    }
    out
}

/// Holder-side attenuation: `requested` must be a known subset of `held`.
/// Equal is allowed (re-issue). Amplification is not.
pub fn attenuate(held: &[String], requested: &[String]) -> Result<Vec<String>, String> {
    if requested.is_empty() {
        return Err("attenuated grant must not be empty".into());
    }
    validate_held(held, requested)?;
    Ok(unique_in_order(requested))
}

/// Progressive grant: start from an optional named set, then optionally
/// restrict further. The result must be ⊆ `held`.
///
/// - `set` only → expand the set (must be ⊆ held).
/// - `set` + `requested` → `requested` ⊆ set ⊆ held.
/// - `requested` only → [`attenuate`].
/// - neither → error (the mint path supplies [`capability::default_minted`]).
pub fn progressive_grant(
    held: &[String],
    set: Option<&str>,
    requested: &[String],
) -> Result<Vec<String>, String> {
    match (set, requested.is_empty()) {
        (Some(name), true) => {
            let expanded = expand_set(name)?;
            validate_held(held, &expanded)?;
            Ok(expanded)
        }
        (Some(name), false) => {
            let expanded = expand_set(name)?;
            validate_held(&expanded, requested)
                .map_err(|e| e.replace("holder grant", "named set"))?;
            attenuate(held, requested)
        }
        (None, false) => attenuate(held, requested),
        (None, true) => Err("grant is empty: pass a capability set or capabilities".into()),
    }
}

/// A derived token cannot outlive its parent. Session holders have no
/// parent expiry, so any future `requested` is allowed. Past `now` fails.
pub fn attenuate_expiry(
    parent: Option<DateTime<Utc>>,
    requested: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, String> {
    if let Some(exp) = requested {
        if exp <= now {
            return Err("expires_at must be in the future".into());
        }
        if let Some(parent_exp) = parent {
            if exp > parent_exp {
                return Err("expires_at cannot exceed the parent token".into());
            }
        }
        return Ok(Some(exp));
    }
    Ok(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{FEDERATION_ADMIN, FEDERATION_INGEST};
    use chrono::Duration;

    #[test]
    fn named_sets_are_known_and_exclude_federation() {
        let sets = named_sets();
        assert_eq!(sets.len(), 2);
        assert_eq!(sets[0].name, AGENT_WORKER);
        assert_eq!(sets[1].name, HUMAN_ADMIN);
        for set in &sets {
            capability::validate_list(&set.capabilities).unwrap();
            assert!(!set.capabilities.iter().any(|c| c == FEDERATION_INGEST));
            assert!(!set.capabilities.iter().any(|c| c == FEDERATION_ADMIN));
        }
        assert!(sets[1].capabilities.iter().any(|c| c == TOKEN_ADMIN));
        assert!(expand_set("maidan.agent.intern").is_err());
    }

    #[test]
    fn worker_is_a_subset_of_admin() {
        let worker = expand_set(AGENT_WORKER).unwrap();
        let admin = expand_set(HUMAN_ADMIN).unwrap();
        for cap in &worker {
            assert!(admin.contains(cap), "{cap} missing from admin");
        }
        assert!(admin.len() > worker.len());
    }

    #[test]
    fn held_sets_requires_the_full_expansion() {
        let worker = expand_set(AGENT_WORKER).unwrap();
        assert_eq!(held_sets(&worker), vec![AGENT_WORKER]);
        assert!(held_sets(&worker[..3]).is_empty());
        let admin = expand_set(HUMAN_ADMIN).unwrap();
        assert_eq!(held_sets(&admin), vec![AGENT_WORKER, HUMAN_ADMIN]);
        assert!(held_sets(&capability::default_minted()).is_empty());
    }

    #[test]
    fn attenuate_drops_rights_and_rejects_amplification() {
        let held = expand_set(AGENT_WORKER).unwrap();
        let weaker = attenuate(
            &held,
            &[
                WORKSPACE_READ.into(),
                MESSAGE_POST.into(),
                WORKSPACE_READ.into(),
            ],
        )
        .unwrap();
        assert_eq!(weaker, vec![WORKSPACE_READ, MESSAGE_POST]);

        let err = attenuate(&held, &[TOKEN_ADMIN.into()]).unwrap_err();
        assert!(err.contains("exceeds holder grant"));
        assert!(attenuate(&held, &[]).is_err());
        assert!(attenuate(&held, &["bogus:cap".into()])
            .unwrap_err()
            .contains("unknown"));
    }

    #[test]
    fn progressive_grant_set_then_restrict() {
        let admin = expand_set(HUMAN_ADMIN).unwrap();
        let full_worker = progressive_grant(&admin, Some(AGENT_WORKER), &[]).unwrap();
        assert_eq!(full_worker, expand_set(AGENT_WORKER).unwrap());

        let restricted = progressive_grant(
            &admin,
            Some(AGENT_WORKER),
            &[WORKSPACE_READ.into(), SEARCH_QUERY.into()],
        )
        .unwrap();
        assert_eq!(restricted, vec![WORKSPACE_READ, SEARCH_QUERY]);

        let err = progressive_grant(&admin, Some(AGENT_WORKER), &[TOKEN_ADMIN.into()]).unwrap_err();
        assert!(err.contains("named set"), "{err}");

        let worker = expand_set(AGENT_WORKER).unwrap();
        assert!(progressive_grant(&worker, Some(HUMAN_ADMIN), &[])
            .unwrap_err()
            .contains("exceeds holder grant"));

        assert!(progressive_grant(&admin, None, &[]).is_err());
        assert_eq!(
            progressive_grant(&admin, None, &[WORKSPACE_READ.into()]).unwrap(),
            vec![WORKSPACE_READ]
        );
    }

    #[test]
    fn expiry_cannot_outlive_the_parent() {
        let now = Utc::now();
        let parent = now + Duration::hours(2);
        let ok = now + Duration::hours(1);
        let late = now + Duration::hours(3);
        assert_eq!(
            attenuate_expiry(Some(parent), Some(ok), now).unwrap(),
            Some(ok)
        );
        assert!(attenuate_expiry(Some(parent), Some(late), now)
            .unwrap_err()
            .contains("cannot exceed"));
        assert_eq!(
            attenuate_expiry(Some(parent), None, now).unwrap(),
            Some(parent)
        );
        assert!(attenuate_expiry(None, Some(now - Duration::minutes(1)), now).is_err());
        assert_eq!(attenuate_expiry(None, Some(ok), now).unwrap(), Some(ok));
    }
}
