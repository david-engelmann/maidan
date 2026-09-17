//! EventKind JSON-Schema pack.
//!
//! ATProto-lexicon analogue. The observable `$type` string **is** the contract
//! (Hyrum's Law home):
//!
//! - new fields are optional
//! - names do not change
//! - unknown fields are ignored
//! - a breaking change is a new type (`/2`), never a silent reshape of `/1`
//!
//! `$type` is injected on **wire envelopes**. Stored `maidan_events.payload`
//! still tags on `kind` (`#[serde(tag = "kind")]`). `Event` does not
//! `deny_unknown_fields`, so extra JSON is ignored on read.
//!
//! The committed pack under `contracts/lexicon/` is the input for a future SDK
//! 0.2 typed model — this crate does not bump the SDK.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::events::{Event, EventKind, StoredEvent};
use crate::ids::{ChannelId, ThreadId, WorkspaceId};
use crate::waiter::{EXAMPLE_PLAN_RESULT_KIND, EXAMPLE_REVIEW_RESULT_KIND, WAITER_RESULT_SCHEMA};
use crate::wasi::{WASI_INVOKE_TYPE, WASI_RESULT_TYPE};

/// JSON Schema dialect the pack is written against.
pub const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Extra (non-event) `$type`s registered in the pack: the frozen waiter
/// envelope, generic namespaced producer examples, and the WASI slash ABI.
pub const EXTRA_TYPE_IDS: &[&str] = &[
    WAITER_RESULT_SCHEMA,
    EXAMPLE_REVIEW_RESULT_KIND,
    EXAMPLE_PLAN_RESULT_KIND,
    WASI_INVOKE_TYPE,
    WASI_RESULT_TYPE,
];

/// File name for a `$type` inside `contracts/lexicon/` (`/` → `.`).
pub fn schema_filename(type_id: &str) -> String {
    format!("{}.json", type_id.replace('/', "."))
}

/// Insert `$type` onto a JSON object. No-op for non-objects.
pub fn inject_type(value: &mut Value, type_id: &str) {
    if let Value::Object(map) = value {
        map.insert("$type".to_string(), Value::String(type_id.to_string()));
    }
}

/// Serialize `event` and stamp `$type` for the wire.
pub fn event_wire(event: &Event) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(event)?;
    inject_type(&mut value, &event.kind().type_id());
    Ok(value)
}

/// Durable columns of [`StoredEvent`] without going through its `Serialize`
/// impl (which calls this helper).
#[derive(Serialize)]
struct StoredEventFields<'a> {
    id: i64,
    lsn: i64,
    kind: EventKind,
    workspace_id: Option<WorkspaceId>,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    payload: &'a Value,
    occurred_at: DateTime<Utc>,
    prev_hash: &'a str,
    content_hash: &'a str,
}

/// Serialize a log row and stamp `$type` from `kind`. Does not rewrite the
/// nested `payload` (that stays the stored `Event` JSON, tagged on `kind`).
pub fn stored_event_wire(event: &StoredEvent) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(StoredEventFields {
        id: event.id,
        lsn: event.lsn,
        kind: event.kind,
        workspace_id: event.workspace_id,
        channel_id: event.channel_id,
        thread_id: event.thread_id,
        payload: &event.payload,
        occurred_at: event.occurred_at,
        prev_hash: &event.prev_hash,
        content_hash: &event.content_hash,
    })?;
    inject_type(&mut value, &event.kind.type_id());
    Ok(value)
}

impl Serialize for StoredEvent {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        stored_event_wire(self)
            .map_err(serde::ser::Error::custom)?
            .serialize(serializer)
    }
}

/// Recursively sort object keys. Arrays keep order. The snapshot canon
/// (`NEW-snapshot-tests`) diffs this shape, not serde's insertion order.
pub fn normalize(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    out.insert(key, normalize(v.clone()));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        other => other,
    }
}

/// Wire event with `$type`, keys sorted.
pub fn normalized_event_wire(event: &Event) -> Result<Value, serde_json::Error> {
    event_wire(event).map(normalize)
}

/// JSON Schema document for one [`EventKind`]. Nested payloads (Message,
/// Thread, …) are snapshot-tested, not fully schematized — `additionalProperties`
/// is true so unknown fields stay ignored.
pub fn event_schema(kind: EventKind) -> Value {
    let type_id = kind.type_id();
    json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "$id": type_id,
        "title": type_id,
        "description": format!(
            "Wire envelope for `{kind}`. `$type` is the contract: new fields optional, no renames, unknown ignored, breaking = new type.",
            kind = kind.as_str()
        ),
        "type": "object",
        "additionalProperties": true,
        "required": ["$type", "kind"],
        "properties": {
            "$type": {
                "type": "string",
                "const": type_id,
            },
            "kind": {
                "type": "string",
                "const": kind.as_str(),
            },
        },
    })
}

fn namespaced_result_schema(type_id: &str, description: &str) -> Value {
    json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "$id": type_id,
        "title": type_id,
        "description": description,
        "type": "object",
        "additionalProperties": true,
        "required": ["result_kind", "status"],
        "properties": {
            "$type": {
                "type": "string",
                "const": type_id,
            },
            "schema": {
                "type": "string",
                "const": type_id,
            },
            "result_kind": { "type": "string" },
            "status": { "type": "string" },
        },
    })
}

/// JSON Schema for the frozen waiter envelope (`maidan.waiter.result/1`).
pub fn waiter_result_schema() -> Value {
    json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "$id": WAITER_RESULT_SCHEMA,
        "title": WAITER_RESULT_SCHEMA,
        "description": "Frozen waiter-result envelope. `schema` and `$type` are the same NSID; either identifies `/1`. Unknown fields ignored. Breaking = new type.",
        "type": "object",
        "additionalProperties": true,
        "required": ["result_kind", "status"],
        "properties": {
            "$type": {
                "type": "string",
                "const": WAITER_RESULT_SCHEMA,
            },
            "schema": {
                "type": "string",
                "const": WAITER_RESULT_SCHEMA,
            },
            "result_kind": { "type": "string" },
            "status": { "type": "string" },
            "deliver_to": { "type": "array" },
            "rendered": { "type": "string" },
            "summary": { "type": "string" },
            "view_url": { "type": "string" },
            "pr": { "type": "string" },
            "head_sha": { "type": "string" },
            "findings": { "type": "array" },
        },
    })
}

/// JSON Schema for `example.review.result/1`.
pub fn example_review_result_schema() -> Value {
    namespaced_result_schema(
        EXAMPLE_REVIEW_RESULT_KIND,
        "Generic example review-result producer shape. A string facet, not a closed enum.",
    )
}

/// JSON Schema for `example.plan.result/1`.
pub fn example_plan_result_schema() -> Value {
    namespaced_result_schema(
        EXAMPLE_PLAN_RESULT_KIND,
        "Generic example plan-result producer shape. A string facet, not a closed enum.",
    )
}

/// JSON Schema for `maidan.slash.wasi-invoke/1`.
pub fn wasi_invoke_schema() -> Value {
    json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "$id": WASI_INVOKE_TYPE,
        "title": WASI_INVOKE_TYPE,
        "description": "Host→guest slash invoke. Stdin JSON. The guest is a tool, not an agent. Unknown fields ignored. Breaking = /2.",
        "type": "object",
        "additionalProperties": true,
        "required": ["$type", "command", "args", "workspace_id", "channel_id", "thread_id", "author_id", "message_id"],
        "properties": {
            "$type": {
                "type": "string",
                "const": WASI_INVOKE_TYPE,
            },
            "command": { "type": "string" },
            "args": { "type": "string" },
            "workspace_id": { "type": "string", "format": "uuid" },
            "channel_id": { "type": "string", "format": "uuid" },
            "thread_id": { "type": "string", "format": "uuid" },
            "author_id": { "type": "string", "format": "uuid" },
            "message_id": { "type": "string", "format": "uuid" },
        },
    })
}

/// JSON Schema for `maidan.slash.wasi-result/1`.
pub fn wasi_result_schema() -> Value {
    json!({
        "$schema": JSON_SCHEMA_DIALECT,
        "$id": WASI_RESULT_TYPE,
        "title": WASI_RESULT_TYPE,
        "description": "WASI slash runtime result. stdout is the guest capture. Fail-closed kinds: fuel_exhausted, memory_limit, trap, banned_import, invalid_module. Unknown fields ignored. Breaking = /2.",
        "type": "object",
        "additionalProperties": true,
        "required": ["$type", "ok"],
        "properties": {
            "$type": {
                "type": "string",
                "const": WASI_RESULT_TYPE,
            },
            "ok": { "type": "boolean" },
            "stdout": { "type": "string" },
            "stderr": { "type": "string" },
            "error": { "type": "string" },
            "error_kind": {
                "type": "string",
                "enum": ["fuel_exhausted", "memory_limit", "trap", "banned_import", "invalid_module"],
            },
        },
    })
}

/// Sorted catalogue of every `$type` in the pack.
pub fn catalog() -> Value {
    let mut types: Vec<String> = EventKind::ALL
        .iter()
        .map(|k| k.type_id())
        .chain(EXTRA_TYPE_IDS.iter().map(|s| (*s).to_string()))
        .collect();
    types.sort();
    json!({
        "description": "Maidan lexicon catalogue. Observable `$type` is the contract. Feeds a future SDK 0.2; this pack does not bump the SDK.",
        "types": types,
    })
}

/// `(filename, pretty JSON)` for every committed pack document, including
/// `catalog.json`. Used by the contract test so a new [`EventKind`] fails
/// until its schema file is added.
pub fn pack_files() -> Vec<(String, String)> {
    let mut files = vec![("catalog.json".to_string(), pretty(&catalog()))];
    for &kind in EventKind::ALL {
        let type_id = kind.type_id();
        files.push((schema_filename(&type_id), pretty(&event_schema(kind))));
    }
    files.push((
        schema_filename(WAITER_RESULT_SCHEMA),
        pretty(&waiter_result_schema()),
    ));
    files.push((
        schema_filename(EXAMPLE_REVIEW_RESULT_KIND),
        pretty(&example_review_result_schema()),
    ));
    files.push((
        schema_filename(EXAMPLE_PLAN_RESULT_KIND),
        pretty(&example_plan_result_schema()),
    ));
    files.push((
        schema_filename(WASI_INVOKE_TYPE),
        pretty(&wasi_invoke_schema()),
    ));
    files.push((
        schema_filename(WASI_RESULT_TYPE),
        pretty(&wasi_result_schema()),
    ));
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn pretty(value: &Value) -> String {
    let normalized = normalize(value.clone());
    let Ok(mut s) = serde_json::to_string_pretty(&normalized) else {
        return String::from("{}\n");
    };
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventKind;

    #[test]
    fn extra_fields_on_an_event_are_ignored() {
        let raw = json!({
            "kind": "thread_result_set",
            "occurred_at": "2023-11-14T22:13:20Z",
            "workspace_id": "00000000-0000-0000-0000-000000000001",
            "channel_id": "00000000-0000-0000-0000-000000000002",
            "thread_id": "00000000-0000-0000-0000-000000000003",
            "produced_by": "00000000-0000-0000-0000-000000000004",
            "brand_new_optional": "ignored",
            "$type": "maidan.event.thread_result_set/1",
        });
        let event: Event = serde_json::from_value(raw).expect("unknown fields ignored");
        assert_eq!(event.kind(), EventKind::ThreadResultSet);
    }

    #[test]
    fn inject_type_does_not_rename_kind() {
        let mut v = json!({"kind": "message_posted", "body": "hi"});
        inject_type(&mut v, "maidan.event.message_posted/1");
        assert_eq!(v["kind"], "message_posted");
        assert_eq!(v["$type"], "maidan.event.message_posted/1");
        assert_eq!(v["body"], "hi");
    }

    #[test]
    fn stored_event_wire_stamps_type_and_keeps_kind() {
        let stored = StoredEvent {
            id: 7,
            lsn: 7,
            kind: EventKind::MessagePosted,
            workspace_id: None,
            channel_id: None,
            thread_id: None,
            payload: json!({"kind": "message_posted", "body": "hi"}),
            occurred_at: chrono::DateTime::parse_from_rfc3339("2023-11-14T22:13:20Z")
                .expect("ts")
                .with_timezone(&chrono::Utc),
            prev_hash: crate::genesis_hash(),
            content_hash: crate::content_hash(&json!({"kind": "message_posted", "body": "hi"}))
                .expect("hash"),
        };
        let wire = stored_event_wire(&stored).expect("wire");
        assert_eq!(wire["$type"], "maidan.event.message_posted/1");
        assert_eq!(wire["kind"], "message_posted");
        assert_eq!(wire["id"], 7);
        assert_eq!(wire["lsn"], 7);
        assert_eq!(wire["prev_hash"], stored.prev_hash);
        assert_eq!(wire["content_hash"], stored.content_hash);
        assert_eq!(wire["payload"]["kind"], "message_posted");
        assert!(
            wire["payload"].get("$type").is_none(),
            "payload stays the stored Event JSON; $type is the row envelope"
        );

        let via_serde = serde_json::to_value(&stored).expect("serde");
        assert_eq!(via_serde["$type"], stored.kind.type_id());
        assert_eq!(via_serde["kind"], "message_posted");

        let mut raw = wire.clone();
        raw["$type"] = json!("maidan.event.message_posted/2");
        raw["unknown_optional"] = json!("ignored");
        let back: StoredEvent = serde_json::from_value(raw).expect("unknown fields ignored");
        assert_eq!(back.kind, EventKind::MessagePosted);
        assert_eq!(back.id, 7);
        assert_eq!(back.payload["kind"], "message_posted");
    }

    #[test]
    fn catalog_lists_every_event_kind_and_waiter_examples() {
        let catalog = catalog();
        let types = catalog["types"].as_array().expect("types");
        let as_str: Vec<&str> = types.iter().filter_map(Value::as_str).collect();
        for &kind in EventKind::ALL {
            assert!(
                as_str.contains(&kind.type_id().as_str()),
                "catalog missing {}",
                kind.type_id()
            );
        }
        for extra in EXTRA_TYPE_IDS {
            assert!(as_str.contains(extra), "catalog missing {extra}");
        }
        let mut sorted = as_str.clone();
        sorted.sort();
        assert_eq!(as_str, sorted);
    }
}
