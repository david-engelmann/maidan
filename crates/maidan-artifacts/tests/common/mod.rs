//! An S3 server for the S3 tests: MinIO under testcontainers.
//!
//! The tests skip only when there is no Docker daemon to talk to. With one, a
//! container that will not start is a failure: the old "skip on any start
//! error" hid that `minio/minio` could no longer be pulled.

use maidan_artifacts::{S3Config, S3Store};
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};

/// MinIO's own images are no longer pullable (Docker Hub, then quay.io), so
/// this is Chainguard's build, pinned by digest like the one in `compose.yaml`.
const IMAGE: &str = "cgr.dev/chainguard/minio";
const DIGEST_TAG: &str =
    "latest@sha256:bd014394a80898e68c149f2311fdf8d5a2c2f3bb2c33b9327ae6d02b4b065ae1";
const API_PORT: u16 = 9000;
const ACCESS_KEY: &str = "maidan";
const SECRET_KEY: &str = "maidan-test-secret";

/// A running S3 server and a store on `bucket`, or `None` when there is no
/// Docker daemon.
pub async fn s3_store(bucket: &str) -> Option<(ContainerAsync<GenericImage>, S3Store)> {
    if !docker_available().await {
        eprintln!("skipping: no Docker daemon");
        return None;
    }
    let container = GenericImage::new(IMAGE, DIGEST_TAG)
        .with_exposed_port(API_PORT.tcp())
        // This build logs to stderr, and prints the API line once it listens.
        .with_wait_for(WaitFor::message_on_stderr("API:"))
        .with_env_var("MINIO_ROOT_USER", ACCESS_KEY)
        .with_env_var("MINIO_ROOT_PASSWORD", SECRET_KEY)
        .with_cmd(["server", "/data"])
        .start()
        .await
        .expect("start the S3 container (Docker is running, so this must work)");
    let port = container
        .get_host_port_ipv4(API_PORT)
        .await
        .expect("S3 API port");
    wait_until_ready(port).await;
    let store = S3Store::new(S3Config {
        endpoint: format!("http://127.0.0.1:{port}"),
        bucket: bucket.to_string(),
        region: "us-east-1".to_string(),
        access_key: ACCESS_KEY.to_string(),
        secret_key: SECRET_KEY.to_string(),
    })
    .await
    .expect("s3 store");
    Some((container, store))
}

/// MinIO prints its `API:` line when it listens, which can be before its
/// object layer is up; a PUT in that window is a `service error` (main, run
/// 38960122644). Its readiness probe answers 200 only once it can serve
/// requests, so the tests start after that instead of racing it.
async fn wait_until_ready(port: u16) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Ok(mut conn) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
            let probe =
                "GET /minio/health/ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
            let mut head = [0u8; 12];
            if conn.write_all(probe.as_bytes()).await.is_ok()
                && conn.read_exact(&mut head).await.is_ok()
                && head.ends_with(b" 200")
            {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "MinIO never reported ready on port {port}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// Pings the daemon testcontainers itself would use, so `DOCKER_HOST` and its
/// rootless and Docker Desktop socket fallbacks count as Docker being there.
async fn docker_available() -> bool {
    match testcontainers::core::client::docker_client_instance().await {
        Ok(docker) => docker.ping().await.is_ok(),
        Err(_) => false,
    }
}
