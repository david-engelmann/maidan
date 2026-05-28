//! S3 multipart upload against MinIO via testcontainers.

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, CompletedPart, S3Config, S3Store};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::minio::MinIO;

#[tokio::test]
async fn s3_multipart_upload_completes_and_content_addresses() {
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
        bucket: "maidan-multipart".to_string(),
        region: "us-east-1".to_string(),
        access_key: "minioadmin".to_string(),
        secret_key: "minioadmin".to_string(),
    })
    .await
    .expect("s3 store");

    // S3 requires every part except the last to be >= 5 MiB; one part is enough
    // to exercise create/upload/complete against MinIO in CI.
    let expected = Bytes::from_static(b"cluster-19 multipart payload");

    let upload = store.begin_multipart_upload().await.expect("begin");
    let etag = store
        .upload_part(&upload, 1, expected.clone())
        .await
        .expect("part 1");
    let sha = store
        .complete_multipart_upload(
            &upload,
            &[CompletedPart {
                part_number: 1,
                etag,
            }],
        )
        .await
        .expect("complete");

    assert!(store.exists(&sha).await.expect("exists"));
    let got = store.get(&sha).await.expect("get");
    assert_eq!(got, expected);
}
