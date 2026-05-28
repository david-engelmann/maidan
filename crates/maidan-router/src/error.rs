use thiserror::Error;

#[derive(Debug, Error)]
pub enum RouterError {
    #[error(transparent)]
    Store(#[from] maidan_store::StoreError),
}
