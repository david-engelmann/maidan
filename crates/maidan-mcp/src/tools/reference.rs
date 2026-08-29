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
