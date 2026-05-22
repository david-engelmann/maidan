//! Concurrent-put stress test for `LocalFsStore`.

use std::sync::Arc;

use bytes::Bytes;
use maidan_artifacts::{ArtifactStore, LocalFsStore, Sha256};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_puts_of_same_content_collapse_to_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(LocalFsStore::new(dir.path()));
    let payload = Bytes::from(vec![0xABu8; 4096]);
    let expected_sha = Sha256::compute(&payload);

    let mut handles = Vec::new();
    for _ in 0..50 {
        let store = store.clone();
        let payload = payload.clone();
        handles.push(tokio::spawn(async move { store.put(payload).await }));
    }

    for handle in handles {
        let sha = handle.await.expect("join").expect("put");
        assert_eq!(sha, expected_sha);
    }

    let got = store.get(&expected_sha).await.expect("get");
    assert_eq!(got, payload);

    let body_files = count_body_files(dir.path());
    assert_eq!(
        body_files, 1,
        "expected exactly one body file after concurrent puts, found {body_files}"
    );

    let leftover_tmps = count_tmp_files(dir.path());
    assert_eq!(
        leftover_tmps, 0,
        "expected no leftover .tmp- files, found {leftover_tmps}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_puts_of_distinct_content_all_persist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(LocalFsStore::new(dir.path()));

    let mut handles = Vec::new();
    for i in 0u32..16 {
        let store = store.clone();
        handles.push(tokio::spawn(async move {
            let bytes = Bytes::from(i.to_le_bytes().to_vec());
            let sha = store.put(bytes.clone()).await?;
            let got = store.get(&sha).await?;
            assert_eq!(got, bytes);
            Ok::<Sha256, maidan_artifacts::ArtifactError>(sha)
        }));
    }

    let mut shas = Vec::new();
    for handle in handles {
        shas.push(handle.await.expect("join").expect("put/get"));
    }
    shas.sort_by_key(|s| s.to_hex());
    shas.dedup();
    assert_eq!(shas.len(), 16, "expected 16 distinct shas");
    assert_eq!(count_body_files(dir.path()), 16);
}

fn count_body_files(root: &std::path::Path) -> usize {
    walk_count(root, |name| !name.starts_with(".tmp-"))
}

fn count_tmp_files(root: &std::path::Path) -> usize {
    walk_count(root, |name| name.starts_with(".tmp-"))
}

fn walk_count(root: &std::path::Path, filter: impl Fn(&str) -> bool + Copy) -> usize {
    let mut n = 0;
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                n += walk_count(&path, filter);
            } else if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if filter(name) {
                    n += 1;
                }
            }
        }
    }
    n
}
