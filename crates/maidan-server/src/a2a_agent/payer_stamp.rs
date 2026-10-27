//! The PayerStamp usage extension (C13). A peer that activates it puts one
//! usage report in its message's `metadata` under [`PAYER_STAMP_EXTENSION`];
//! Maidan records it in the same claim-fenced ledger as `POST
//! /threads/{id}/usage`, against the thread the message's `contextId` names,
//! and answers the stamp in the task's metadata under the same key. The
//! report is the REST body unchanged, so the claim lease still fences it.

use maidan_a2a::{A2aError, A2aErrorKind, Message as A2aMessage};
use maidan_auth::capability::THREAD_TRANSITION;
use maidan_auth::{AuthContext, ThreadScope};
use maidan_store::StoreError;
use maidan_types::{AccountedUsageRequest, ThreadId, UsageLedgerEntry};
use serde_json::Value;

use super::error::{denied, internal, store};
use crate::state::AppState;

/// The extension's URI, as the Agent Card advertises it and as the key of
/// the report in message metadata and of the stamp in task metadata.
pub const PAYER_STAMP_EXTENSION: &str =
    "https://github.com/david-engelmann/maidan/blob/main/docs/Protocols.md#payerstamp-usage-extension-v1";

/// The usage report a message carries, if any, parsed as the REST body.
pub(super) fn report(message: &A2aMessage) -> Result<Option<AccountedUsageRequest>, A2aError> {
    let Some(value) = message
        .metadata
        .as_ref()
        .and_then(|m| m.get(PAYER_STAMP_EXTENSION))
    else {
        return Ok(None);
    };
    serde_json::from_value(integral(value.clone()))
        .map(Some)
        .map_err(|e| A2aError::invalid_params(format!("{PAYER_STAMP_EXTENSION}: {e}")))
}

/// The largest magnitude a double holds every integer up to, exactly.
const EXACT_DOUBLE_INTEGERS: f64 = 9_007_199_254_740_992.0;

/// `value` with every finite whole double within the exact range turned into
/// an integer. Over gRPC the metadata is a protobuf `Struct`, whose numbers
/// are all doubles, so a report's `0` arrives as `0.0`; a fractional or
/// out-of-range number stays a double and the report's integer fields still
/// refuse it.
fn integral(value: Value) -> Value {
    match value {
        Value::Number(n) => match n.as_f64() {
            Some(f)
                if !n.is_i64()
                    && !n.is_u64()
                    && f.is_finite()
                    && f.fract() == 0.0
                    && f.abs() <= EXACT_DOUBLE_INTEGERS =>
            {
                Value::from(f as i64)
            }
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(integral).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, integral(v))).collect())
        }
        other => other,
    }
}

/// Record the report against the context's thread. The reporter is the
/// token's member and the payer the thread's workspace, as on REST.
pub(super) async fn record(
    state: &AppState,
    auth: &AuthContext,
    scope: ThreadScope,
    report: AccountedUsageRequest,
) -> Result<UsageLedgerEntry, A2aError> {
    auth.require_capability(THREAD_TRANSITION).map_err(denied)?;
    let (entry, stored) = state
        .store
        .report_accounted_usage(&report.into_new(scope.thread_id, auth.member_id))
        .await
        .map_err(|err| match err {
            StoreError::ClaimLost(m) | StoreError::Conflict(m) => {
                A2aError::new(A2aErrorKind::InvalidParams, m)
            }
            StoreError::NotFound => A2aError::invalid_params("no such claim on this context"),
            err => store(err),
        })?;
    for stored in stored {
        crate::routes::publish_stored(state, stored).await;
    }
    Ok(entry)
}

/// The stamp a recorded report answers with, under the extension's key.
pub(super) fn stamp(entry: &UsageLedgerEntry) -> Result<Value, A2aError> {
    serde_json::to_value(entry).map_err(internal)
}

/// The cache-affinity key for a context: the thread it binds to, or that
/// thread's parent when it has one, so a child lands where the parent's
/// prefix is warm. The same rule as the SDKs' `routing_hints` (C11).
pub(super) async fn affinity_key(
    state: &AppState,
    thread_id: ThreadId,
) -> Result<String, A2aError> {
    let thread = state.store.get_thread(thread_id).await.map_err(store)?;
    Ok(thread.parent_thread_id.unwrap_or(thread_id).0.to_string())
}

/// The message's metadata without the usage report, which belongs to the
/// ledger rather than the transcript.
pub(super) fn without_report(metadata: &Value) -> Value {
    let mut out = metadata.clone();
    if let Some(map) = out.as_object_mut() {
        map.remove(PAYER_STAMP_EXTENSION);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_a2a::{Part, Role};
    use serde_json::json;

    fn message(metadata: Option<Value>) -> A2aMessage {
        A2aMessage {
            message_id: "m1".into(),
            context_id: None,
            task_id: None,
            role: Role::User,
            parts: vec![Part::text("hi")],
            metadata,
            extensions: vec![],
            reference_task_ids: vec![],
        }
    }

    #[test]
    fn a_message_without_the_extension_carries_no_report() {
        assert!(report(&message(None)).unwrap().is_none());
        assert!(report(&message(Some(json!({"other": 1}))))
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_malformed_report_is_refused_as_invalid_params() {
        let bad = message(Some(json!({ (PAYER_STAMP_EXTENSION): {"model": "m"} })));
        let err = report(&bad).unwrap_err();
        assert_eq!(err.kind, A2aErrorKind::InvalidParams);
    }

    fn stamped(tokens_input: Value) -> A2aMessage {
        message(Some(json!({ (PAYER_STAMP_EXTENSION): {
            "usage_report_id": "0190f0f0-0000-7000-8000-000000000001",
            "claim_lease_id": "0190f0f0-0000-7000-8000-000000000002",
            "model": "peer/model",
            "tokens": { "input": tokens_input, "output": 0.0, "cache_read": 0.0,
                        "cache_write_5m": 0.0, "cache_write_1h": 0.0 },
            "usd_micros": 0.0,
            "price_snapshot": {
                "input_usd_micros_per_million": 0.0,
                "output_usd_micros_per_million": 0.0,
                "cache_read_usd_micros_per_million": 0.0,
                "cache_write_5m_usd_micros_per_million": 0.0,
                "cache_write_1h_usd_micros_per_million": 0.0
            },
            "turns": 1.0,
            "evidence": {}
        }})))
    }

    #[test]
    fn a_report_whose_whole_numbers_arrive_as_doubles_is_read_as_integers() {
        let parsed = report(&stamped(json!(120.0))).unwrap().unwrap();
        assert_eq!(parsed.tokens.input, 120);
        assert_eq!(parsed.turns, 1);
    }

    #[test]
    fn a_fractional_or_out_of_range_double_is_still_refused() {
        for bad in [json!(1.5), json!(1e300), json!(-1e17)] {
            let err = report(&stamped(bad.clone())).unwrap_err();
            assert_eq!(err.kind, A2aErrorKind::InvalidParams, "{bad}");
        }
    }

    #[test]
    fn the_report_is_kept_out_of_the_stored_metadata() {
        let stored = without_report(&json!({ (PAYER_STAMP_EXTENSION): {}, "keep": true }));
        assert_eq!(stored, json!({ "keep": true }));
    }
}
