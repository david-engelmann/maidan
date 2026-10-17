use thiserror::Error;

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("not found")]
    NotFound,

    #[error("invalid sha: {0}")]
    InvalidSha(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("storage backend error: {0}")]
    Storage(String),
}

impl ArtifactError {
    /// A storage error from an S3 SDK call. The SDK's own `Display` stops at
    /// "service error", which left a red CI run with no S3 error code to go
    /// on; the full source chain carries the code and the server's message.
    pub(crate) fn sdk<E: std::error::Error>(op: &str, err: &E) -> Self {
        Self::Storage(format!(
            "{op}: {}",
            aws_sdk_s3::error::DisplayErrorContext(err)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::ArtifactError;
    use aws_sdk_s3::error::SdkError;

    #[test]
    fn an_sdk_error_keeps_its_source_chain() {
        let cause = std::io::Error::other("XMinioStorageFull: minimum free drive threshold");
        let err: SdkError<std::io::Error, ()> = SdkError::construction_failure(cause);
        let ArtifactError::Storage(msg) = ArtifactError::sdk("put_object", &err) else {
            panic!("an SDK failure is a storage error");
        };
        assert!(msg.starts_with("put_object: "), "{msg}");
        assert!(msg.contains("XMinioStorageFull"), "{msg}");
    }
}
