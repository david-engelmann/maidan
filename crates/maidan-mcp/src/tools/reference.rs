//! Cross-thread/message reference tool handler.

use std::sync::Arc;

use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct AddReferenceArgs {
    src_kind: RefSide,
    src_id: uuid::Uuid,
    dst_kind: RefSide,
    dst_id: uuid::Uuid,
    /// Snake_case string; controlled set `RelationKind::CONTROLLED`, unknown → `Other`.
    relation: RelationKind,
}

pub(super) async fn add_reference(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: AddReferenceArgs = serde_json::from_value(args.clone())?;
    let r = store
        .add_reference(NewReference {
            src_kind: a.src_kind,
            src_id: a.src_id,
            dst_kind: a.dst_kind,
            dst_id: a.dst_id,
            relation: a.relation,
        })
        .await?;
    Ok(content_json(&r))
}

#[derive(Deserialize)]
struct ListReferencesArgs {
    src_kind: Option<RefSide>,
    src_id: Option<uuid::Uuid>,
    dst_kind: Option<RefSide>,
    dst_id: Option<uuid::Uuid>,
    /// Optional relation filter (a `RelationKind` wire string, e.g. `refutes`).
    relation: Option<RelationKind>,
}

/// List references FROM a source (forward) OR TO a target (reverse — "what
/// references this"), optionally filtered by relation (Cluster 320). Exactly one of
/// the `src_*` / `dst_*` pairs is required.
pub(super) async fn list_references(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListReferencesArgs = serde_json::from_value(args.clone())?;
    let mut refs = match (a.src_kind, a.src_id, a.dst_kind, a.dst_id) {
        (Some(sk), Some(si), None, None) => store.list_references_from(sk, si).await?,
        (None, None, Some(dk), Some(di)) => store.list_references_to(dk, di).await?,
        _ => {
            return Err(McpError::InvalidParams(
                "provide exactly one of the src_kind+src_id or dst_kind+dst_id pair".into(),
            ))
        }
    };
    if let Some(relation) = a.relation {
        refs.retain(|r| r.relation == relation);
    }
    Ok(content_json(&refs))
}
