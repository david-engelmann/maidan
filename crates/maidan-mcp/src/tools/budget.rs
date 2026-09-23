//! Budget-envelope tool handlers: set/read a thread's budget, report usage
//! (stopping the run on exceed), and read a channel's DLQ.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{
    AccountedUsageRequest, BudgetLimits, ChannelId, ClaimLeaseId, PriceSnapshot, ThreadId,
    TokenUsage,
};
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

/// Unknown fields are rejected — see [`maidan_types::BudgetLimits`]. A typo'd
/// dimension would otherwise read as an omission, and an omission on this call
/// means "remove that limit".
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetBudgetArgs {
    thread_id: uuid::Uuid,
    // The four dimensions are named here rather than `#[serde(flatten)]`-ing a
    // `BudgetPatch`, because **flatten silently defeats `deny_unknown_fields`**
    // — measured, not assumed: with flatten, `{"max_wall_seconds": 5}` parsed
    // clean. That would have undone the unknown-field rejection on the very
    // struct it was added for, where a typo'd dimension disarms a cap.
    //
    // `Option<Option<i64>>` keeps absent distinguishable from an explicit
    // `null`, which is what lets a replace refuse a partial body and a patch
    // leave the rest alone.
    #[serde(default, deserialize_with = "double_option")]
    max_tokens: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    max_usd_micros: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    max_turns: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    max_wall_secs: Option<Option<i64>>,
}

/// Tell "absent" from "explicitly null" — serde collapses both for a plain
/// `Option<T>`. Mirrors the helper on `BudgetPatch`.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

impl SetBudgetArgs {
    fn patch(&self) -> maidan_types::BudgetPatch {
        maidan_types::BudgetPatch {
            max_tokens: self.max_tokens,
            max_usd_micros: self.max_usd_micros,
            max_turns: self.max_turns,
            max_wall_secs: self.max_wall_secs,
        }
    }
}

/// Replace a thread's whole budget envelope.
///
/// A replace means every dimension it does not name becomes "no cap", and a
/// dimension with no cap never binds — so omitting one silently removed a
/// limit. Every dimension must now be stated; `null` is how you say "no cap
/// here". Use `update_thread_budget` to change one without restating the rest.
///
/// Accumulated usage is preserved. Thread access is enforced pre-dispatch.
pub(super) async fn set_thread_budget(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetBudgetArgs = serde_json::from_value(args.clone())?;
    let patch = a.patch();
    let missing = patch.missing_dimensions();
    if !missing.is_empty() {
        return Err(McpError::InvalidParams(format!(
            "a budget replace must state every dimension; missing: {}. \
             Send null to leave one uncapped, or use update_thread_budget to \
             change only some.",
            missing.join(", ")
        )));
    }
    let budget = store
        .set_thread_budget(ThreadId(a.thread_id), patch.apply(BudgetLimits::default()))
        .await?;
    Ok(content_json(&budget))
}

/// Change only the budget dimensions the call names.
///
/// Absent leaves a dimension alone; an explicit `null` clears its cap. Widening
/// therefore always requires saying so, and two orchestrators adjusting
/// different dimensions do not have to coordinate — the merge happens in one
/// transaction rather than as a read-modify-write.
pub(super) async fn update_thread_budget(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetBudgetArgs = serde_json::from_value(args.clone())?;
    let budget = store
        .patch_thread_budget(ThreadId(a.thread_id), a.patch())
        .await?;
    Ok(content_json(&budget))
}

#[derive(Deserialize)]
struct ThreadIdArg {
    thread_id: uuid::Uuid,
}

/// A thread's budget, or null until one is set / usage is first reported.
pub(super) async fn get_thread_budget(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArg = serde_json::from_value(args.clone())?;
    let budget = store.get_thread_budget(ThreadId(a.thread_id)).await?;
    Ok(content_json(&budget))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportUsageArgs {
    thread_id: uuid::Uuid,
    usage_report_id: uuid::Uuid,
    claim_lease_id: uuid::Uuid,
    model: String,
    tokens: TokenUsage,
    usd_micros: i64,
    price_snapshot: PriceSnapshot,
    #[serde(default)]
    turns: i64,
}

impl ReportUsageArgs {
    fn request(self) -> AccountedUsageRequest {
        AccountedUsageRequest {
            usage_report_id: self.usage_report_id,
            claim_lease_id: ClaimLeaseId(self.claim_lease_id),
            model: self.model,
            tokens: self.tokens,
            usd_micros: self.usd_micros,
            price_snapshot: self.price_snapshot,
            turns: self.turns,
        }
    }
}

/// Report incremental usage against a thread's budget — the claim-holder's
/// heartbeat. If it pushes a claimed thread over budget the run is STOPPED
/// (claim released, `ClaimFailed` emitted, DLQ entry recorded); the returned
/// `stopped`/`reason` say so. Thread access is enforced pre-dispatch.
pub(super) async fn report_usage(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReportUsageArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let reporter = if auth.bypass {
        server
            .store
            .get_thread(thread_id)
            .await?
            .assignee_id
            .ok_or_else(|| McpError::InvalidParams("thread has no active claim holder".into()))?
    } else {
        auth.member_id
    };
    let (report, stored) = server
        .store
        .report_accounted_usage(&a.request().into_new(thread_id, reporter))
        .await?;
    for stored in stored {
        server.publish_stored(&stored).await;
    }
    Ok(content_json(&report))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDlqArgs {
    channel_id: uuid::Uuid,
    #[serde(default)]
    limit: Option<i64>,
}

/// A channel's agent-work dead-letter queue — runs stopped for exceeding their
/// budget, newest first. Channel access is enforced pre-dispatch.
pub(super) async fn list_dlq(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListDlqArgs = serde_json::from_value(args.clone())?;
    let limit = a.limit.unwrap_or(50).clamp(1, 200);
    let entries = store
        .list_channel_dlq(ChannelId(a.channel_id), limit)
        .await?;
    Ok(content_json(&entries))
}

#[cfg(test)]
mod budget_args_tests {
    use super::*;

    /// Re-proved after the patch semantics reshaped this struct. **This exact
    /// assertion failed** when the fields were `#[serde(flatten)]`-ed: flatten
    /// silently defeats `deny_unknown_fields`, so a typo'd dimension parsed
    /// clean and disarmed a cap again.
    #[test]
    fn a_typod_dimension_is_still_rejected() {
        let bad = r#"{"thread_id":"00000000-0000-0000-0000-000000000001","max_wall_seconds":5}"#;
        assert!(
            serde_json::from_str::<SetBudgetArgs>(bad).is_err(),
            "a misspelled dimension must not be absorbed"
        );
    }

    /// Absent and null stay distinguishable through the args struct, not just
    /// through `BudgetPatch` — this is the layer the MCP surface actually parses.
    #[test]
    fn absent_and_null_survive_the_args_layer() {
        let id = r#""00000000-0000-0000-0000-000000000001""#;
        let absent: SetBudgetArgs =
            serde_json::from_str(&format!(r#"{{"thread_id":{id}}}"#)).unwrap();
        let null: SetBudgetArgs =
            serde_json::from_str(&format!(r#"{{"thread_id":{id},"max_tokens":null}}"#)).unwrap();
        assert!(absent.patch().is_empty(), "absent names nothing");
        assert_eq!(null.patch().max_tokens, Some(None), "null is explicit");
        assert_eq!(
            absent.patch().missing_dimensions().len(),
            4,
            "a replace would refuse this body"
        );
    }
}
