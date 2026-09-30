//! The opaque `pageToken`s the list operations hand out. A client can send
//! any string back, so decoding is public and pure, and fuzzed
//! (`fuzz/fuzz_targets/a2a_page_token.rs`).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chrono::{DateTime, SecondsFormat, Utc};

use crate::protocol::A2aError;

/// A `ListTasks` page position: the status timestamp and id of the last task
/// shown.
pub type TaskCursor = (DateTime<Utc>, String);

fn invalid() -> A2aError {
    A2aError::invalid_params("invalid pageToken")
}

/// The token for the `ListTasks` page after `cursor`. Timestamps are kept to
/// the millisecond, the precision the listing sorts by.
pub fn encode_task_cursor((at, id): &TaskCursor) -> String {
    URL_SAFE_NO_PAD.encode(format!(
        "{}|{id}",
        at.to_rfc3339_opts(SecondsFormat::Millis, true)
    ))
}

pub fn decode_task_cursor(token: &str) -> Result<TaskCursor, A2aError> {
    let raw = URL_SAFE_NO_PAD.decode(token).map_err(|_| invalid())?;
    let raw = String::from_utf8(raw).map_err(|_| invalid())?;
    let (at, id) = raw.split_once('|').ok_or_else(invalid)?;
    let at = DateTime::parse_from_rfc3339(at).map_err(|_| invalid())?;
    Ok((at.with_timezone(&Utc), id.to_string()))
}

/// The token for the push-config page after the config `config_id`.
pub fn encode_config_cursor(config_id: &str) -> String {
    URL_SAFE_NO_PAD.encode(config_id)
}

pub fn decode_config_cursor(token: &str) -> Result<String, A2aError> {
    let raw = URL_SAFE_NO_PAD.decode(token).map_err(|_| invalid())?;
    String::from_utf8(raw).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::A2aErrorKind;

    #[test]
    fn task_cursors_round_trip_and_reject_garbage() {
        let at = DateTime::parse_from_rfc3339("2026-09-28T12:00:00.123Z")
            .unwrap()
            .with_timezone(&Utc);
        let cursor = (at, "0192-task".to_string());
        assert_eq!(
            decode_task_cursor(&encode_task_cursor(&cursor)).unwrap(),
            cursor
        );
        for bad in ["!!", "bm8tc2VwYXJhdG9y", "bm90LWEtdGltZXxpZA"] {
            assert_eq!(
                decode_task_cursor(bad).unwrap_err().kind,
                A2aErrorKind::InvalidParams
            );
        }
    }

    #[test]
    fn config_cursors_round_trip_and_reject_garbage() {
        assert_eq!(
            decode_config_cursor(&encode_config_cursor("cfg|1")).unwrap(),
            "cfg|1"
        );
        assert_eq!(
            decode_config_cursor("!!").unwrap_err().kind,
            A2aErrorKind::InvalidParams
        );
    }
}
