//! Kind-aware helpers that pair object-store writes with [`NewArtifact`] metadata.

use bytes::Bytes;
use maidan_types::{ArtifactKind, NewArtifact};

use crate::error::ArtifactError;
use crate::sha::Sha256;
use crate::store::ArtifactStore;

fn new_meta(sha: &Sha256, kind: ArtifactKind, mime_type: &str, bytes: &Bytes) -> NewArtifact {
    NewArtifact {
        sha256: sha.to_string(),
        size_bytes: bytes.len() as i64,
        mime_type: Some(mime_type.to_string()),
        kind,
        uploaded_by: None,
    }
}

pub async fn put_screenshot(
    store: &dyn ArtifactStore,
    bytes: Bytes,
) -> Result<(Sha256, NewArtifact), ArtifactError> {
    let sha = store.put(bytes.clone()).await?;
    Ok((
        sha,
        new_meta(
            &sha,
            ArtifactKind::Screenshot,
            ArtifactKind::Screenshot.default_mime(),
            &bytes,
        ),
    ))
}

pub async fn put_recording(
    store: &dyn ArtifactStore,
    bytes: Bytes,
) -> Result<(Sha256, NewArtifact), ArtifactError> {
    let sha = store.put(bytes.clone()).await?;
    Ok((
        sha,
        new_meta(
            &sha,
            ArtifactKind::Recording,
            ArtifactKind::Recording.default_mime(),
            &bytes,
        ),
    ))
}

pub async fn put_transcript(
    store: &dyn ArtifactStore,
    bytes: Bytes,
) -> Result<(Sha256, NewArtifact), ArtifactError> {
    let sha = store.put(bytes.clone()).await?;
    Ok((
        sha,
        new_meta(
            &sha,
            ArtifactKind::Transcript,
            ArtifactKind::Transcript.default_mime(),
            &bytes,
        ),
    ))
}

pub async fn put_code_dump(
    store: &dyn ArtifactStore,
    bytes: Bytes,
) -> Result<(Sha256, NewArtifact), ArtifactError> {
    let sha = store.put(bytes.clone()).await?;
    Ok((
        sha,
        new_meta(
            &sha,
            ArtifactKind::CodeDump,
            ArtifactKind::CodeDump.default_mime(),
            &bytes,
        ),
    ))
}

pub async fn put_attachment(
    store: &dyn ArtifactStore,
    bytes: Bytes,
) -> Result<(Sha256, NewArtifact), ArtifactError> {
    let sha = store.put(bytes.clone()).await?;
    Ok((
        sha,
        new_meta(
            &sha,
            ArtifactKind::Attachment,
            ArtifactKind::Attachment.default_mime(),
            &bytes,
        ),
    ))
}
