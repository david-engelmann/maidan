//! Round-trip integration tests for `LocalFsStore`.

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, LocalFsStore, Sha256};

fn store() -> (LocalFsStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalFsStore::new(dir.path());
    (store, dir)
}

#[tokio::test]
async fn put_get_round_trip() {
    let (store, _dir) = store();
    let payload = Bytes::from_static(b"hello world");
    let sha = store.put(payload.clone()).await.expect("put");
    assert_eq!(sha, Sha256::compute(&payload));

    let got = store.get(&sha).await.expect("get");
    assert_eq!(got, payload);
}

#[tokio::test]
async fn exists_reports_correctly() {
    let (store, _dir) = store();
    let sha = Sha256::compute(b"foo");
    assert!(!store.exists(&sha).await.unwrap());
    store.put(Bytes::from_static(b"foo")).await.unwrap();
    assert!(store.exists(&sha).await.unwrap());
}

#[tokio::test]
async fn get_missing_returns_not_found() {
    let (store, _dir) = store();
    let sha = Sha256::compute(b"nope");
    let err = store.get(&sha).await.unwrap_err();
    assert!(matches!(err, maidan_artifacts::ArtifactError::NotFound));
}

#[tokio::test]
async fn put_same_content_twice_dedups() {
    let (store, dir) = store();
    let payload = Bytes::from_static(b"identical");
    let sha_a = store.put(payload.clone()).await.unwrap();
    let sha_b = store.put(payload.clone()).await.unwrap();
    assert_eq!(sha_a, sha_b);

    // exactly one body file on disk
    let count = count_body_files(dir.path());
    assert_eq!(count, 1, "expected one body file, found {count}");
}

#[tokio::test]
async fn delete_removes_body() {
    let (store, _dir) = store();
    let payload = Bytes::from_static(b"deleteme");
    let sha = store.put(payload).await.unwrap();
    assert!(store.exists(&sha).await.unwrap());
    store.delete(&sha).await.unwrap();
    assert!(!store.exists(&sha).await.unwrap());
    let err = store.get(&sha).await.unwrap_err();
    assert!(matches!(err, maidan_artifacts::ArtifactError::NotFound));
}

#[tokio::test]
async fn delete_missing_is_not_found() {
    let (store, _dir) = store();
    let sha = Sha256::compute(b"nope");
    let err = store.delete(&sha).await.unwrap_err();
    assert!(matches!(err, maidan_artifacts::ArtifactError::NotFound));
}

#[tokio::test]
async fn body_path_uses_fanout() {
    let (store, dir) = store();
    let sha = store
        .put(Bytes::from_static(b"check fanout"))
        .await
        .unwrap();
    let hex = sha.to_hex();
    let expected = dir.path().join(&hex[0..2]).join(&hex[2..4]).join(&hex[4..]);
    assert!(
        expected.exists(),
        "expected fanout path {expected:?} to exist"
    );
}

fn count_body_files(root: &std::path::Path) -> usize {
    fn walk(p: &std::path::Path) -> usize {
        let mut n = 0;
        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    n += walk(&path);
                } else if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if !name.starts_with(".tmp-") {
                        n += 1;
                    }
                }
            }
        }
        n
    }
    walk(root)
}
