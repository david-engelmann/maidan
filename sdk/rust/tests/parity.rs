//! The SDK method map (`../methods.json`) against this client, offline: every
//! example is called and must send the request the map names, and its reply
//! must decode with no member the models leave out. A method missing from this
//! crate does not compile here; `scripts/check-sdk-parity.py` checks the names
//! of the rest of the map and runs the same examples through Python and
//! TypeScript.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

use maidan::{
    Client, DelegateOptions, Footprint, FootprintItem, FootprintKind, FootprintMode, MaidanError,
    ReviewDecision, ReviewOptions,
};
use serde_json::{json, Value};

const METHODS: &str = include_str!("../../methods.json");

#[derive(Debug, PartialEq)]
struct Seen {
    method: String,
    path: String,
    body: Value,
}

/// Answer one request with `status` and `body`, and report what was sent.
fn serve_once(status: u16, body: Value) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut len = 0usize;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).unwrap();
            if h == "\r\n" || h.is_empty() {
                break;
            }
            let (name, value) = h.split_once(':').unwrap();
            if name.eq_ignore_ascii_case("content-length") {
                len = value.trim().parse().unwrap();
            }
        }
        let mut raw = vec![0; len];
        reader.read_exact(&mut raw).unwrap();
        let sent = if raw.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&raw).unwrap()
        };
        tx.send(Seen {
            method,
            path,
            body: sent,
        })
        .unwrap();
        let out = if status == 204 {
            "HTTP/1.1 204 No Content\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string()
        } else {
            let text = body.to_string();
            format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                text.len()
            )
        };
        stream.write_all(out.as_bytes()).unwrap();
    });
    (format!("http://{addr}"), rx)
}

/// Make the example's call; the answer is the members its reply carried that
/// the models leave out.
fn call(c: &Client, op: &str) -> Result<Vec<String>, MaidanError> {
    Ok(match op {
        "reviews.packet" => c.reviews().packet("t1")?.unknown_members(),
        "reviews.request" => {
            c.reviews().request("t1", "m2")?;
            Vec::new()
        }
        "reviews.submit" => c
            .reviews()
            .submit(
                "t1",
                ReviewDecision::RequestChanges,
                &ReviewOptions {
                    note: Some("the retry test still sleeps"),
                    ..Default::default()
                },
            )?
            .unknown_members(),
        "reviews.list" => c
            .reviews()
            .list("t1")?
            .iter()
            .flat_map(|r| r.unknown_members())
            .collect(),
        "reviews.status" => c.reviews().status("t1")?.unknown_members(),
        "approval_gates.list" => c
            .approval_gates()
            .list("w1")?
            .iter()
            .flat_map(|g| g.unknown_members())
            .collect(),
        "approval_gates.answer" => c
            .approval_gates()
            .answer("g1", "ab12", "decline", None)?
            .unknown_members(),
        "threads.set_footprint" => {
            let footprint = Footprint::new(
                "acme/widgets",
                FootprintMode::Exclusive,
                vec![FootprintItem::new(FootprintKind::Path, "sdk/go")],
            );
            c.threads()
                .set_footprint("t1", &footprint)?
                .map(|f| f.unknown_members())
                .unwrap_or_default()
        }
        "threads.get_footprint" => c.threads().get_footprint("t1")?.unknown_members(),
        "delegation_grants.create" => c
            .delegation_grants()
            .create(
                "w1",
                "m1",
                "m2",
                &["workspace:read"],
                "triage the inbox",
                "2026-11-12T00:00:00Z",
            )?
            .unknown_members(),
        "delegation_grants.revoke" => c.delegation_grants().revoke("w1", "dg1")?.unknown_members(),
        "tokens.delegate" => c
            .tokens()
            .delegate(
                "dg1",
                &["workspace:read"],
                &DelegateOptions {
                    label: Some("inbox triage"),
                    ..Default::default()
                },
            )?
            .unknown_members(),
        "tokens.attenuate" => c
            .tokens()
            .attenuate(
                &["workspace:read"],
                &DelegateOptions {
                    expires_at: Some("2026-11-11T06:00:00Z"),
                    ..Default::default()
                },
            )?
            .unknown_members(),
        other => panic!("{other} has an example in the method map but no call in tests/parity.rs"),
    })
}

#[test]
fn every_example_sends_the_mapped_request_and_decodes_its_reply() {
    let map: Value = serde_json::from_str(METHODS).unwrap();
    let mut called = 0;
    for m in map["methods"].as_array().unwrap() {
        let Some(ex) = m.get("example") else {
            continue;
        };
        let op = m["op"].as_str().unwrap();
        let status = ex["reply"]["status"].as_u64().unwrap() as u16;
        let (base, seen) = serve_once(status, ex["reply"]["body"].clone());
        let c = Client::new(base, "t").with_max_retries(0);
        let unknown = call(&c, op).unwrap_or_else(|e| panic!("{op}: {e}"));
        assert!(unknown.is_empty(), "{op}: the models leave out {unknown:?}");
        let want = Seen {
            method: ex["request"]["method"].as_str().unwrap().to_string(),
            path: ex["request"]["path"].as_str().unwrap().to_string(),
            body: ex["request"]["body"].clone(),
        };
        assert_eq!(seen.recv().unwrap(), want, "{op}");
        called += 1;
    }
    assert!(called > 0, "the method map has no examples");
}

#[test]
fn a_gate_answer_carries_its_content_when_given() {
    let (base, seen) = serve_once(200, json!(null));
    let c = Client::new(base, "t").with_max_retries(0);
    let content = json!({ "reason": "wrong release" });
    // The reply is not a gate; only the request matters here.
    let _ = c
        .approval_gates()
        .answer("g1", "ab12", "decline", Some(&content));
    let got = seen.recv().unwrap();
    assert_eq!(got.body["content"], content);
}
