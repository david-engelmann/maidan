//! An object store that accepts a connection and never answers fails the
//! request within the read timeout, instead of holding it forever.

use std::time::{Duration, Instant};

use maidan_artifacts::{S3Config, S3Store};

#[tokio::test]
async fn a_store_that_never_answers_fails_within_the_read_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for socket in listener.incoming() {
            held.push(socket);
        }
    });
    let config = S3Config {
        endpoint: format!("http://{addr}"),
        bucket: "maidan".into(),
        region: "us-east-1".into(),
        access_key: "key".into(),
        secret_key: "secret".into(),
    };
    let started = Instant::now();
    let outcome =
        S3Store::with_timeouts(config, Duration::from_secs(1), Duration::from_millis(500)).await;
    let waited = started.elapsed();
    assert!(outcome.is_err(), "a silent store is an error, not a hang");
    assert!(
        waited < Duration::from_secs(20),
        "bounded by the read timeout and the SDK's retries, took {waited:?}"
    );
}
