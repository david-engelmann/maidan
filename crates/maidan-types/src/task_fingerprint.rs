//! Task fingerprints (Context Economics C8): a hash of a task's brief and
//! inputs, so a second, identical task can be linked to the first instead of
//! being run again.
//!
//! The fingerprint covers what a worker is asked to do: the title, the
//! description, the kind, the engine, the threat-model flag, the acceptance
//! text and the Linear issue. Bookkeeping that does not change the work (the
//! size estimate, the review tier, the priority, the footprint) is left out.
//! Text is trimmed; nothing else is normalized, so two briefs that differ
//! by a word never share a fingerprint. Each field is length-prefixed, so no
//! two different briefs encode to the same bytes.

use sha2::{Digest, Sha256};

use crate::TaskAttributes;

/// The fingerprint's version. It is part of the hashed bytes, so a change to
/// what is covered gives every task a new fingerprint rather than a false
/// match with an old one.
pub const TASK_FINGERPRINT_VERSION: &str = "maidan-task-fingerprint-v1";

/// The fingerprint of a task's brief and inputs: 64 lowercase hex characters
/// of SHA-256. `None` when the task has neither a title nor a description,
/// because there is nothing to compare.
pub fn task_fingerprint(
    title: Option<&str>,
    description: Option<&str>,
    attributes: &TaskAttributes,
) -> Option<String> {
    let title = title.map(str::trim).filter(|t| !t.is_empty());
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    if title.is_none() && description.is_none() {
        return None;
    }
    let mut hasher = Sha256::new();
    field(&mut hasher, Some(TASK_FINGERPRINT_VERSION));
    field(&mut hasher, title);
    field(&mut hasher, description);
    field(&mut hasher, Some(attributes.kind.as_str()));
    field(&mut hasher, attributes.engine.map(|e| e.as_str()));
    field(
        &mut hasher,
        Some(if attributes.threat_model { "1" } else { "0" }),
    );
    field(&mut hasher, attributes.acceptance.as_deref().map(str::trim));
    field(&mut hasher, attributes.linear_issue.as_deref());
    Some(
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// One field: an absence marker, or a presence marker, the byte length and
/// the bytes.
fn field(hasher: &mut Sha256, value: Option<&str>) {
    match value {
        None => hasher.update([0u8]),
        Some(value) => {
            hasher.update([1u8]);
            hasher.update((value.len() as u64).to_be_bytes());
            hasher.update(value.as_bytes());
        }
    }
}

/// A task's stored fingerprint, and the earlier open task with the same
/// fingerprint in the same workspace that it was linked to when created.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskFingerprint {
    pub fingerprint: String,
    /// The oldest live task in the workspace that was open or in review
    /// with the same fingerprint when this one was created. `None` when
    /// there was none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<crate::ThreadId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TaskEngine, TaskKind};

    fn fp(title: Option<&str>, description: Option<&str>, a: &TaskAttributes) -> String {
        task_fingerprint(title, description, a).unwrap()
    }

    #[test]
    fn the_same_brief_gives_the_same_fingerprint_and_trims() {
        let a = TaskAttributes::default();
        let one = fp(Some("Fix the sweep"), Some("details"), &a);
        assert_eq!(one, fp(Some("  Fix the sweep\n"), Some("details "), &a));
        assert_eq!(one.len(), 64);
        assert!(one
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn every_covered_field_changes_the_fingerprint() {
        let base = TaskAttributes::default();
        let one = fp(Some("t"), Some("d"), &base);
        assert_ne!(one, fp(Some("t2"), Some("d"), &base));
        assert_ne!(one, fp(Some("t"), Some("d2"), &base));
        assert_ne!(one, fp(Some("t"), None, &base));
        for changed in [
            TaskAttributes {
                kind: TaskKind::Review,
                ..base.clone()
            },
            TaskAttributes {
                engine: Some(TaskEngine::parse("pi").unwrap()),
                ..base.clone()
            },
            TaskAttributes {
                threat_model: true,
                ..base.clone()
            },
            TaskAttributes {
                acceptance: Some("tests pass".into()),
                ..base.clone()
            },
            TaskAttributes {
                linear_issue: Some("ENG-1".into()),
                ..base.clone()
            },
        ] {
            assert_ne!(one, fp(Some("t"), Some("d"), &changed), "{changed:?}");
        }
    }

    #[test]
    fn bookkeeping_fields_do_not_change_it() {
        let base = TaskAttributes::default();
        let sized = TaskAttributes {
            size: crate::TaskSize::parse("S"),
            tier: crate::TaskTier::parse("A"),
            ..base.clone()
        };
        assert_eq!(fp(Some("t"), None, &base), fp(Some("t"), None, &sized));
    }

    #[test]
    fn field_boundaries_cannot_be_shifted() {
        let a = TaskAttributes::default();
        assert_ne!(fp(Some("ab"), Some("c"), &a), fp(Some("a"), Some("bc"), &a));
        assert_ne!(fp(Some("t"), None, &a), fp(None, Some("t"), &a));
    }

    #[test]
    fn a_task_with_no_brief_has_no_fingerprint() {
        let a = TaskAttributes::default();
        assert_eq!(task_fingerprint(None, None, &a), None);
        assert_eq!(task_fingerprint(Some("  "), Some(""), &a), None);
    }
}
