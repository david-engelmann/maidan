//! S3Store round trip against MinIO via testcontainers.

mod common;

use bytes::Bytes;
use maidan_artifacts::ArtifactStore;

#[tokio::test]
async fn s3_store_round_trips_bytes_against_minio() {
    let Some((_container, store)) = common::s3_store("maidan-artifacts").await else {
        return;
    };

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
