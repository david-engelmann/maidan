//! Write-time rules for typed references (`add_reference`).
//!
//! There is no per-relation kind matrix: any thread/message pair is allowed,
//! including [`RelationKind::Other`]. The store still refuses a self-link, a
//! missing end, a cross-workspace link, and a cycle along `depends`,
//! `supersedes`, or `duplicates`. Every refusal is [`StoreError::InvalidInput`].

use maidan_types::{NewReference, RelationKind, WorkspaceId};

use crate::error::StoreError;

/// Relations that must stay acyclic. Other controlled kinds, and [`RelationKind::Other`],
/// may form cycles.
pub(crate) fn is_acyclic_relation(relation: &RelationKind) -> bool {
    matches!(
        relation,
        RelationKind::Depends | RelationKind::Supersedes | RelationKind::Duplicates
    )
}

/// Walk existing edges of the same relation, starting from the new edge's
/// target. If the source is already reachable, adding the edge would close a
/// cycle. `$1`–`$5` bind on both backends: target kind, target id, relation,
/// source kind, source id.
pub(crate) const CYCLE_SQL: &str = "WITH RECURSIVE reachable(node_kind, node_id) AS (
    SELECT dst_kind, dst_id
      FROM maidan_references
     WHERE src_kind = $1 AND src_id = $2 AND relation = $3
    UNION
    SELECT r.dst_kind, r.dst_id
      FROM maidan_references r
      JOIN reachable x ON r.src_kind = x.node_kind AND r.src_id = x.node_id
     WHERE r.relation = $3
)
SELECT 1 AS hit FROM reachable WHERE node_kind = $4 AND node_id = $5 LIMIT 1";

/// Why a new reference was refused. Maps to [`StoreError::InvalidInput`] (REST
/// 400 / MCP InvalidParams), the same as a task-dependency refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReferenceRuleError {
    SelfLink,
    MissingSource,
    MissingTarget,
    CrossWorkspace,
    Cycle,
}

impl ReferenceRuleError {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::SelfLink => "a reference cannot point at itself",
            Self::MissingSource => "reference source does not exist",
            Self::MissingTarget => "reference target does not exist",
            Self::CrossWorkspace => "a reference cannot cross workspaces",
            Self::Cycle => "adding this reference would create a cycle",
        }
    }
}

impl From<ReferenceRuleError> for StoreError {
    fn from(err: ReferenceRuleError) -> Self {
        Self::InvalidInput(err.as_str().into())
    }
}

/// Check the resolved facts for `new`. Callers pass `would_cycle` only after
/// walking the same-relation graph; it must be `false` when the relation is
/// not acyclic, or when either end is missing or in another workspace.
pub(crate) fn check(
    new: &NewReference,
    src_workspace: Option<WorkspaceId>,
    dst_workspace: Option<WorkspaceId>,
    would_cycle: bool,
) -> Result<(), StoreError> {
    if new.src_kind == new.dst_kind && new.src_id == new.dst_id {
        return Err(ReferenceRuleError::SelfLink.into());
    }
    let src = src_workspace.ok_or(ReferenceRuleError::MissingSource)?;
    let dst = dst_workspace.ok_or(ReferenceRuleError::MissingTarget)?;
    if src != dst {
        return Err(ReferenceRuleError::CrossWorkspace.into());
    }
    if would_cycle {
        return Err(ReferenceRuleError::Cycle.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use maidan_types::{NewReference, RefSide, RelationKind, WorkspaceId};
    use uuid::Uuid;

    use super::{check, ReferenceRuleError};
    use crate::error::StoreError;

    fn ws(n: u128) -> WorkspaceId {
        WorkspaceId(Uuid::from_u128(n))
    }

    fn edge(src: u128, dst: u128) -> NewReference {
        NewReference {
            src_kind: RefSide::Thread,
            src_id: Uuid::from_u128(src),
            dst_kind: RefSide::Thread,
            dst_id: Uuid::from_u128(dst),
            relation: RelationKind::Depends,
        }
    }

    fn input(err: ReferenceRuleError, result: Result<(), StoreError>) {
        assert_eq!(result, Err(StoreError::InvalidInput(err.as_str().into())));
    }

    #[test]
    fn refuses_a_self_link_before_other_facts() {
        input(
            ReferenceRuleError::SelfLink,
            check(&edge(1, 1), None, None, false),
        );
    }

    #[test]
    fn refuses_a_missing_source_or_target() {
        input(
            ReferenceRuleError::MissingSource,
            check(&edge(1, 2), None, Some(ws(1)), false),
        );
        input(
            ReferenceRuleError::MissingTarget,
            check(&edge(1, 2), Some(ws(1)), None, false),
        );
    }

    #[test]
    fn refuses_a_cross_workspace_link() {
        input(
            ReferenceRuleError::CrossWorkspace,
            check(&edge(1, 2), Some(ws(1)), Some(ws(2)), false),
        );
    }

    #[test]
    fn refuses_a_cycle() {
        input(
            ReferenceRuleError::Cycle,
            check(&edge(1, 2), Some(ws(1)), Some(ws(1)), true),
        );
    }

    #[test]
    fn accepts_a_same_workspace_edge() {
        assert_eq!(check(&edge(1, 2), Some(ws(1)), Some(ws(1)), false), Ok(()));
    }
}
