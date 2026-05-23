//! Streaming put via `put_reader` handles payloads larger than typical buffers.

use std::io::Cursor;

use bytes::Bytes;
use maidan_artifacts::{put_reader, ArtifactStore, LocalFsStore};

const EIGHT_MIB: usize = 8 * 1024 * 1024;

#[tokio::test]
async fn put_reader_accepts_multi_mib_payload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = LocalFsStore::new(dir.path());
    let payload: Vec<u8> = (0..EIGHT_MIB + 1).map(|i| (i % 251) as u8).collect();
    let sha = put_reader(&store, Cursor::new(payload.clone()))
        .await
        .expect("put_reader");
    let got = store.get(&sha).await.expect("get");
    assert_eq!(got.len(), EIGHT_MIB + 1);
    assert_eq!(got, Bytes::from(payload));
}
