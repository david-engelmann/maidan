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

async fn docker_available() -> bool {
    match testcontainers::bollard::Docker::connect_with_defaults() {
        Ok(docker) => docker.ping().await.is_ok(),
        Err(_) => false,
    }
}
