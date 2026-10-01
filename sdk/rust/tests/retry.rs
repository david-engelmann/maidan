//! Retries, idempotency keys and auto-paging against a scripted local HTTP
//! server (no Maidan server needed).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use maidan::{retry_delay, Client, MaidanError, StoredEvent, MAX_PAGE_SIZE};
use serde_json::{json, Value};

enum Reply {
    Status(u16, Value, Vec<(&'static str, &'static str)>),
    Hangup,
}

#[derive(Debug, Clone)]
struct Seen {
    target: String,
    key: Option<String>,
}

fn serve(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<Seen>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    std::thread::spawn(move || {
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let (mut key, mut len) = (None, 0usize);
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h == "\r\n" || h.is_empty() {
                    break;
                }
                let (name, value) = h.split_once(':').unwrap();
                let value = value.trim().to_string();
                match name.to_ascii_lowercase().as_str() {
                    "idempotency-key" => key = Some(value),
                    "content-length" => len = value.parse().unwrap(),
                    _ => {}
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            log.lock().unwrap().push(Seen { target, key });
            match reply {
                Reply::Hangup => drop(stream),
                Reply::Status(code, body, headers) => {
                    let body = body.to_string();
                    let mut out = format!(
                        "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                        body.len()
                    );
                    for (k, v) in headers {
                        out.push_str(&format!("{k}: {v}\r\n"));
                    }
                    out.push_str("\r\n");
                    out.push_str(&body);
                    stream.write_all(out.as_bytes()).unwrap();
                }
            }
        }
    });
    (format!("http://{addr}"), seen)
}

fn client(base: &str) -> (Client, Arc<Mutex<Vec<Duration>>>) {
    let sleeps = Arc::new(Mutex::new(Vec::new()));
    let s = sleeps.clone();
    let c = Client::new(base, "t").with_sleep(move |d| s.lock().unwrap().push(d));
    (c, sleeps)
}

fn ok(code: u16, body: Value) -> Reply {
    Reply::Status(code, body, vec![])
}

const TS: &str = "2026-09-29T00:00:00Z";

fn message(id: &str) -> Value {
    json!({"id": id, "thread_id": "t1", "author_id": "m", "body": "hi", "posted_at": TS})
}

fn channel(id: &str) -> Value {
    json!({"id": id, "workspace_id": "w", "name": "n", "private": false, "created_at": TS, "updated_at": TS})
}

fn thread(id: &str) -> Value {
    json!({"id": id, "channel_id": "ch", "state": "open", "created_at": TS, "updated_at": TS})
}

fn event(id: i64) -> Value {
    json!({
        "$type": "maidan.event.message_posted/1", "id": id, "lsn": id, "kind": "message_posted",
        "payload": {}, "occurred_at": TS, "prev_hash": "p", "content_hash": "c"
    })
}

#[test]
fn a_write_retries_a_lost_response_with_the_same_key() {
    let (base, seen) = serve(vec![Reply::Hangup, ok(201, message("m1"))]);
    let (c, _) = client(&base);
    assert_eq!(c.messages().post("t1", "hi").unwrap().id, "m1");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    assert!(seen[0].key.is_some());
    assert_eq!(seen[0].key, seen[1].key);
}

#[test]
fn each_write_gets_its_own_key_and_reads_get_none() {
    let (base, seen) = serve(vec![
        ok(201, message("a")),
        ok(201, message("b")),
        ok(200, json!([])),
    ]);
    let (c, _) = client(&base);
    c.messages().post("t1", "a").unwrap();
    c.messages().post("t1", "b").unwrap();
    c.channels().list("w").unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_ne!(seen[0].key, seen[1].key);
    assert_eq!(seen[2].key, None);
}

#[test]
fn rate_limit_then_server_errors_are_retried_a_bounded_number_of_times() {
    let (base, seen) = serve(vec![
        Reply::Status(429, json!({}), vec![("retry-after", "3")]),
        ok(503, json!({})),
        ok(503, json!({"detail": "down"})),
    ]);
    let (c, sleeps) = client(&base);
    let err = c.channels().list("w").unwrap_err();
    assert_eq!(err.status(), 503);
    assert_eq!(seen.lock().unwrap().len(), 3);
    let sleeps = sleeps.lock().unwrap().clone();
    assert_eq!(sleeps[0], Duration::from_secs(3));
    assert!(sleeps[1] >= Duration::from_millis(500) && sleeps[1] <= Duration::from_secs(1));
}

#[test]
fn a_409_in_flight_is_retried_and_a_plain_409_is_not() {
    let (base, seen) = serve(vec![
        ok(
            409,
            json!({"type": "https://maidan.dev/problems/idempotency-key-in-flight"}),
        ),
        ok(201, channel("c")),
    ]);
    let (c, _) = client(&base);
    assert_eq!(c.channels().create("w", "n", false).unwrap().id, "c");
    assert_eq!(seen.lock().unwrap().len(), 2);

    let (base, seen) = serve(vec![ok(
        409,
        json!({"type": "https://maidan.dev/problems/conflict"}),
    )]);
    let (c, _) = client(&base);
    assert!(matches!(
        c.channels().create("w", "n", false).unwrap_err(),
        MaidanError::Conflict(_)
    ));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn a_403_is_not_retried_and_zero_retries_turns_them_off() {
    let (base, seen) = serve(vec![ok(403, json!({}))]);
    let (c, _) = client(&base);
    assert_eq!(c.channels().list("w").unwrap_err().status(), 403);
    assert_eq!(seen.lock().unwrap().len(), 1);
    let (base, seen) = serve(vec![ok(503, json!({}))]);
    let (c, _) = client(&base);
    let c = c.with_max_retries(0);
    assert_eq!(c.channels().list("w").unwrap_err().status(), 503);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn retry_delay_is_capped_exponential_with_jitter() {
    assert_eq!(retry_delay(0, None, 0.0), Duration::from_millis(250));
    assert_eq!(retry_delay(0, None, 1.0), Duration::from_millis(500));
    assert_eq!(retry_delay(10, None, 1.0), Duration::from_secs(8));
    assert_eq!(retry_delay(0, Some("120"), 0.0), Duration::from_secs(60));
}

#[test]
fn threads_list_all_pages_by_cursor() {
    let (base, seen) = serve(vec![
        ok(200, json!([thread("a"), thread("b")])),
        ok(200, json!([thread("c")])),
    ]);
    let (c, _) = client(&base);
    let ids: Vec<String> = c
        .threads()
        .list_all("ch", 2)
        .map(|t| t.unwrap().id)
        .collect();
    assert_eq!(ids, ["a", "b", "c"]);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0].target, "/channels/ch/threads?limit=2");
    assert_eq!(seen[1].target, "/channels/ch/threads?limit=2&cursor=b");
}

#[test]
fn list_events_all_pages_by_after_id() {
    let (base, seen) = serve(vec![
        ok(200, json!([event(1), event(2)])),
        ok(200, json!([event(3)])),
    ]);
    let (c, _) = client(&base);
    let events: Vec<StoredEvent> = c
        .list_events_all("w", &[("limit", "2")])
        .collect::<Result<_, _>>()
        .unwrap();
    let n = events.len();
    assert_eq!(n, 3);
    assert!(seen.lock().unwrap()[1].target.contains("after_id=2"));
}

/// The server clamps `limit` to 500. A helper that asked for more would get a
/// page of 500, read it as short, and stop with rows left.
#[test]
fn paging_helpers_ask_for_no_more_than_the_servers_page_size() {
    let full: Vec<Value> = (0..MAX_PAGE_SIZE)
        .map(|i| thread(&format!("t{i}")))
        .collect();
    let (base, seen) = serve(vec![ok(200, json!(full)), ok(200, json!([thread("last")]))]);
    let (c, _) = client(&base);
    let threads: Vec<_> = c
        .threads()
        .list_all("ch", 1000)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(threads.len(), MAX_PAGE_SIZE + 1);
    assert_eq!(
        seen.lock().unwrap()[0].target,
        "/channels/ch/threads?limit=500"
    );

    let full: Vec<Value> = (1..=MAX_PAGE_SIZE as i64).map(event).collect();
    let (base, seen) = serve(vec![
        ok(200, json!(full)),
        ok(200, json!([event(MAX_PAGE_SIZE as i64 + 1)])),
    ]);
    let (c, _) = client(&base);
    let events: Vec<StoredEvent> = c
        .list_events_all("w", &[("limit", "1000")])
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(events.len(), MAX_PAGE_SIZE + 1);
    assert!(seen.lock().unwrap()[0].target.contains("limit=500"));
}
