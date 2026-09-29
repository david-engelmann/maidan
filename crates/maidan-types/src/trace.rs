//! W3C Trace Context (`traceparent` / `tracestate`).
//!
//! A trace is transport metadata. It is not part of an event's content hash
//! and it is not a field of any domain event. Callers that do not send a
//! header get a new trace; a header that does not parse is ignored rather
//! than rejected.

use serde::{Deserialize, Serialize};

/// One span in a W3C trace: the ids a `traceparent` header names, plus an
/// optional `tracestate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceContext {
    trace_id: [u8; 16],
    span_id: [u8; 8],
    /// The W3C flags byte. Bit 0 is the sampled flag.
    flags: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracestate: Option<String>,
}

impl TraceContext {
    /// A new root span, sampled, with no `tracestate`.
    pub fn root() -> Self {
        Self {
            trace_id: random_id(),
            span_id: random_span(),
            flags: 0x01,
            tracestate: None,
        }
    }

    /// Parse a `traceparent` header. `None` when it is not a traceparent this
    /// process can continue (bad shape, version `ff`, or an all-zero id).
    /// A version other than `00` is accepted for the fields this version
    /// already defines; what we emit is always version `00`.
    ///
    /// `tracestate` that is empty, too long, or contains a control character
    /// is dropped. The trace itself is kept.
    pub fn parse(traceparent: &str, tracestate: Option<&str>) -> Option<Self> {
        let raw = traceparent.trim();
        if raw.len() < 55 || raw.len() > 200 {
            return None;
        }
        let mut parts = raw.split('-');
        let version = parts.next()?;
        let trace_id = parts.next()?;
        let span_id = parts.next()?;
        let flags = parts.next()?;
        if version.len() != 2 || trace_id.len() != 32 || span_id.len() != 16 || flags.len() < 2 {
            return None;
        }
        if version.eq_ignore_ascii_case("ff") {
            return None;
        }
        let _version = u8::from_str_radix(version, 16).ok()?;
        let trace_id = parse_hex_16(trace_id)?;
        let span_id = parse_hex_8(span_id)?;
        let flags = u8::from_str_radix(&flags[..2], 16).ok()?;
        if trace_id == [0; 16] || span_id == [0; 8] {
            return None;
        }
        Some(Self {
            trace_id,
            span_id,
            flags,
            tracestate: tracestate.and_then(clean_tracestate),
        })
    }

    /// The columns as stored. `None` when the traceparent does not parse, so
    /// a corrupt row cannot break a read.
    pub fn from_columns(traceparent: Option<String>, tracestate: Option<String>) -> Option<Self> {
        let traceparent = traceparent?;
        Self::parse(&traceparent, tracestate.as_deref())
    }

    /// A child span in this trace. The trace id, flags and `tracestate` stay;
    /// the span id is new. This is the server span under an incoming header,
    /// and the parent an outbound call names.
    pub fn child(&self) -> Self {
        Self {
            trace_id: self.trace_id,
            span_id: random_span(),
            flags: self.flags,
            tracestate: self.tracestate.clone(),
        }
    }

    /// `version-traceid-spanid-flags`, lowercase, version `00`.
    pub fn traceparent(&self) -> String {
        format!(
            "00-{}-{}-{:02x}",
            hex_encode(&self.trace_id),
            hex_encode(&self.span_id),
            self.flags
        )
    }

    pub fn tracestate(&self) -> Option<&str> {
        self.tracestate.as_deref()
    }

    pub fn trace_id(&self) -> &[u8; 16] {
        &self.trace_id
    }

    pub fn span_id(&self) -> &[u8; 8] {
        &self.span_id
    }

    pub fn sampled(&self) -> bool {
        self.flags & 0x01 == 0x01
    }

    /// Keep `tracestate` when an exported span replaces the ids.
    pub fn with_tracestate(mut self, tracestate: Option<String>) -> Self {
        self.tracestate = tracestate;
        self
    }

    /// Rebuild from an exported span's ids. `tracestate` is kept by the caller.
    pub fn from_ids(
        trace_id: [u8; 16],
        span_id: [u8; 8],
        sampled: bool,
        tracestate: Option<String>,
    ) -> Option<Self> {
        if trace_id == [0; 16] || span_id == [0; 8] {
            return None;
        }
        Some(Self {
            trace_id,
            span_id,
            flags: if sampled { 0x01 } else { 0x00 },
            tracestate,
        })
    }
}

fn clean_tracestate(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 512 {
        return None;
    }
    if raw.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(raw.to_string())
}

fn random_id() -> [u8; 16] {
    *uuid::Uuid::new_v4().as_bytes()
}

fn random_span() -> [u8; 8] {
    let bytes = uuid::Uuid::new_v4();
    let mut span = [0u8; 8];
    span.copy_from_slice(&bytes.as_bytes()[..8]);
    if span == [0; 8] {
        span[0] = 1;
    }
    span
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn parse_hex_16(s: &str) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    parse_hex_into(s, &mut out)?;
    Some(out)
}

fn parse_hex_8(s: &str) -> Option<[u8; 8]> {
    let mut out = [0u8; 8];
    parse_hex_into(s, &mut out)?;
    Some(out)
}

fn parse_hex_into(s: &str, out: &mut [u8]) -> Option<()> {
    if s.len() != out.len() * 2 {
        return None;
    }
    let bytes = s.as_bytes();
    for (i, slot) in out.iter_mut().enumerate() {
        let hi = from_hex(bytes[i * 2])?;
        let lo = from_hex(bytes[i * 2 + 1])?;
        *slot = (hi << 4) | lo;
    }
    Some(())
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_traceparent_round_trips_and_a_child_keeps_the_trace() {
        let raw = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let parsed = TraceContext::parse(raw, Some("vendor=one")).expect("parse");
        assert_eq!(parsed.traceparent(), raw);
        assert_eq!(parsed.tracestate(), Some("vendor=one"));
        assert!(parsed.sampled());
        let child = parsed.child();
        assert_eq!(child.trace_id(), parsed.trace_id());
        assert_ne!(child.span_id(), parsed.span_id());
        assert_eq!(child.tracestate(), parsed.tracestate());
        assert!(child
            .traceparent()
            .starts_with("00-4bf92f3577b34da6a3ce929d0e0e4736-"));
    }

    #[test]
    fn uppercase_hex_is_accepted_and_emitted_lowercase() {
        let parsed = TraceContext::parse(
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00F067AA0BA902B7-01",
            None,
        )
        .expect("parse");
        assert_eq!(
            parsed.traceparent(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
    }

    #[test]
    fn zero_ids_version_ff_and_garbage_are_not_a_trace() {
        assert!(TraceContext::parse(
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            None
        )
        .is_none());
        assert!(TraceContext::parse(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            None
        )
        .is_none());
        assert!(TraceContext::parse(
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            None
        )
        .is_none());
        assert!(TraceContext::parse("not-a-trace", None).is_none());
        assert!(TraceContext::parse("", None).is_none());
    }

    #[test]
    fn a_bad_tracestate_is_dropped_not_the_trace() {
        let parsed = TraceContext::parse(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            Some("has\na newline"),
        )
        .expect("parse");
        assert!(parsed.tracestate().is_none());
    }

    #[test]
    fn a_future_version_keeps_the_known_fields() {
        let parsed = TraceContext::parse(
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
            None,
        )
        .expect("parse");
        assert_eq!(
            parsed.traceparent(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
    }

    #[test]
    fn an_unsampled_flag_is_preserved_on_the_child() {
        let parsed = TraceContext::parse(
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00",
            None,
        )
        .expect("parse");
        assert!(!parsed.sampled());
        assert!(!parsed.child().sampled());
    }
}
