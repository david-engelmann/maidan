//! `POST /a2a/v1/rpc` takes JSON-RPC from any A2A client. The body is read as
//! JSON, its envelope checked, and its params decoded as the method's request
//! type. An envelope reprinted as JSON reads back the same, and so do decoded
//! params; an error keeps an id only when the request carried that id.
#![no_main]

use libfuzzer_sys::fuzz_target;
use maidan_a2a::{parse_envelope, parse_operation, JsonRpcId, Operation};
use serde::Serialize;
use serde_json::{json, Value};

fn params_of(operation: &Operation) -> Value {
    fn to<T: Serialize>(request: &T) -> Value {
        serde_json::to_value(request).expect("a request serializes")
    }
    match operation {
        Operation::SendMessage(r) | Operation::SendStreamingMessage(r) => to(r),
        Operation::GetTask(r) => to(r),
        Operation::ListTasks(r) => to(r),
        Operation::CancelTask(r) => to(r),
        Operation::SubscribeToTask(r) => to(r),
        Operation::CreatePushNotificationConfig(r) => to(r),
        Operation::GetPushNotificationConfig(r) => to(r),
        Operation::ListPushNotificationConfigs(r) => to(r),
        Operation::DeletePushNotificationConfig(r) => to(r),
        Operation::GetExtendedAgentCard => Value::Null,
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(body) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    let sent_id = body.get("id").cloned();
    let envelope = match parse_envelope(body) {
        Ok(envelope) => envelope,
        Err((id, _)) => {
            let echoed = match &id {
                JsonRpcId::Null => return,
                JsonRpcId::Number(n) => json!(n),
                JsonRpcId::Str(s) => json!(s),
            };
            assert_eq!(sent_id, Some(echoed), "an error answered an id not sent");
            return;
        }
    };
    let reprinted = json!({
        "jsonrpc": "2.0",
        "id": envelope.id,
        "method": envelope.method,
        "params": envelope.params,
    });
    let again = parse_envelope(reprinted.clone())
        .unwrap_or_else(|(_, err)| panic!("{reprinted} does not read back: {err:?}"));
    assert_eq!(again, envelope);

    let Ok(operation) = parse_operation(&envelope.method, envelope.params) else {
        return;
    };
    let params = params_of(&operation);
    let decoded = parse_operation(&envelope.method, params.clone()).unwrap_or_else(|err| {
        panic!(
            "{} params {params} do not read back: {err:?}",
            envelope.method
        )
    });
    assert_eq!(decoded, operation, "{params} is not stable");
});
