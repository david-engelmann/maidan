//! Black-box test for the Maidan Rust client against a running server (MAIDAN_URL,
//! auth disabled). Run via `scripts/sdk-test.sh rust`, which boots a server. Each
//! test skips (returns) when MAIDAN_URL is unset, matching the repo's Docker-skip
//! convention. These scenarios also exercise the server's REST + WS surface.

use std::sync::mpsc;
use std::time::Duration;

use maidan::Client;
use serde_json::{json, Value};

fn base() -> Option<String> {
    std::env::var("MAIDAN_URL").ok()
}

// Member creation isn't in the SDK surface (seeded via bootstrap/CLI); seed one
// over the raw bootstrap route.
fn seed(c: &Client, base: &str) -> (Value, Value, Value, Value) {
    let ws = c.workspaces().create("rust-sdk").unwrap();
    let wid = ws["id"].as_str().unwrap();
    let member: Value = ureq::post(&format!("{base}/workspaces/{wid}/members"))
        .send_json(json!({ "handle": "sdk-agent", "kind": "agent" }))
        .unwrap()
        .into_json()
        .unwrap();
    let channel = c.channels().create(wid, "general", false).unwrap();
    let thread = c
        .threads()
        .create(channel["id"].as_str().unwrap(), "kickoff")
        .unwrap();
    (ws, member, channel, thread)
}

#[test]
fn hero_loop_post_list_context() {
    let Some(base) = base() else {
        eprintln!("skip: MAIDAN_URL unset");
        return;
    };
    let c = Client::new(&base, std::env::var("MAIDAN_TOKEN").unwrap_or_default());
    let (_ws, member, _ch, thread) = seed(&c, &base);
    let tid = thread["id"].as_str().unwrap();
    c.messages()
        .post(
            tid,
            member["id"].as_str().unwrap(),
            "hello from the rust sdk",
        )
        .unwrap();
    let msgs = c.messages().list(tid, &[]).unwrap();
    assert!(msgs
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["body"] == "hello from the rust sdk"));
    assert!(c.threads().context(tid, &[]).unwrap().is_object());
}

#[test]
fn get_result_unset_is_404() {
    // A full set_result round-trip needs a real produced_by member (auth-enabled;
    // the server's thread_result_e2e proves it). Under the auth-disabled harness the
    // acting member is nil, so exercise the result route + client error path.
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, "");
    let (_ws, _m, _ch, thread) = seed(&c, &base);
    let err = c
        .threads()
        .get_result(thread["id"].as_str().unwrap())
        .unwrap_err();
    assert_eq!(err.status, 404);
}

#[test]
fn errors_surface_status() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, "");
    let err = c
        .threads()
        .get("00000000-0000-0000-0000-000000000000")
        .unwrap_err();
    assert!(err.status >= 400);
}

#[test]
fn claim_next_returns_claimable_or_null() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, "");
    let (_ws, member, ch, _t) = seed(&c, &base);
    let _res = c
        .claim_next_thread(
            ch["id"].as_str().unwrap(),
            json!({ "member_id": member["id"] }),
        )
        .unwrap();
}

#[test]
fn subscribe_delivers_a_message() {
    let Some(base) = base() else {
        return;
    };
    let c = Client::new(&base, "");
    let (ws, member, _ch, thread) = seed(&c, &base);
    let (tx, rx) = mpsc::channel();
    let tid = thread["id"].as_str().unwrap().to_string();
    let sub = c
        .subscribe(
            json!({ "workspace_id": ws["id"], "kinds": ["message_posted"] }),
            move |e| {
                if e["thread_id"].as_str() == Some(tid.as_str()) {
                    let _ = tx.send(e);
                }
            },
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(200)); // let the subscription attach
    c.messages()
        .post(
            thread["id"].as_str().unwrap(),
            member["id"].as_str().unwrap(),
            "ws ping",
        )
        .unwrap();
    let e = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("did not receive the message_posted event");
    assert_eq!(e["kind"], "message_posted");
    sub.close();
}
