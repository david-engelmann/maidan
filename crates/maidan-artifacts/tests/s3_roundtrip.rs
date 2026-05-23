//! S3Store integration test against MinIO via testcontainers.

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, S3Config, S3Store};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::minio::MinIO;

#[tokio::test]
async fn s3_store_round_trips_bytes_against_minio() {
    let container = match MinIO::default().start().await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };

    let port = container
        .get_host_port_ipv4(9000)
        .await
        .expect("minio port");
    let endpoint = format!("http://127.0.0.1:{port}");

    let store = S3Store::new(S3Config {
        endpoint,
        bucket: "maidan-artifacts".to_string(),
        region: "us-east-1".to_string(),
        access_key: "minioadmin".to_string(),
        secret_key: "minioadmin".to_string(),
    })
    .await
    .expect("s3 store");

    let payload = Bytes::from_static(b"cluster-e s3 substrate");
    let sha = store.put(payload.clone()).await.expect("put");
    assert!(store.exists(&sha).await.expect("exists"));
    let got = store.get(&sha).await.expect("get");
    assert_eq!(got, payload);

    let sha2 = store.put(payload).await.expect("dedup put");
    assert_eq!(sha, sha2);

    store.delete(&sha).await.expect("delete");
    assert!(!store.exists(&sha).await.expect("exists after delete"));
    assert!(store.get(&sha).await.is_err());
}
