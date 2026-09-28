//! S3 multipart upload against MinIO via testcontainers.

mod common;

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, CompletedPart};

#[tokio::test]
async fn s3_multipart_upload_completes_and_content_addresses() {
    let Some((_container, store)) = common::s3_store("maidan-multipart").await else {
        return;
    };

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
