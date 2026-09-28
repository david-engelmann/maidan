//! Maidan failures as A2A errors (§3.3.2).

use maidan_a2a::{A2aError, A2aErrorKind};
use maidan_auth::AuthError;
use maidan_store::StoreError;

/// A caller that lacks a capability or may not act where it asked.
pub(super) fn denied(err: AuthError) -> A2aError {
    match err {
        AuthError::Forbidden(message) => A2aError::new(A2aErrorKind::PermissionDenied, message),
        AuthError::Unauthorized => A2aError::new(
            A2aErrorKind::PermissionDenied,
            "missing or invalid bearer token",
        ),
        AuthError::Store(err) => store(err),
    }
}

/// A task lookup whose access check failed. A task the caller may not read is
/// reported exactly like one that does not exist, so ids never leak (§3.1.3).
pub(super) fn hidden(task_id: &str, err: AuthError) -> A2aError {
    match err {
        AuthError::Forbidden(_)
        | AuthError::Unauthorized
        | AuthError::Store(StoreError::NotFound) => A2aError::task_not_found(task_id),
        AuthError::Store(err) => store(err),
    }
}

/// A store failure. Refusals the caller can act on keep their message;
/// anything else is logged and reported without internals.
pub(super) fn store(err: StoreError) -> A2aError {
    match err {
        StoreError::InvalidInput(message) => A2aError::invalid_params(message),
        StoreError::SpawnRejected(denial) => {
            A2aError::new(A2aErrorKind::PermissionDenied, denial.to_string())
        }
        err => internal(err),
    }
}

pub(super) fn internal(err: impl std::fmt::Display) -> A2aError {
    tracing::error!(error = %err, "a2a operation failed");
    A2aError::internal("internal error")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inaccessible_task_reads_as_missing() {
        for err in [
            AuthError::Forbidden("private channel".into()),
            AuthError::Store(StoreError::NotFound),
        ] {
            let mapped = hidden("t1", err);
            assert_eq!(mapped.kind, A2aErrorKind::TaskNotFound);
            assert_eq!(mapped.metadata["taskId"], "t1");
        }
    }

    #[test]
    fn internals_do_not_leak() {
        let mapped = store(StoreError::Conflict("uq_secret_index".into()));
        assert_eq!(mapped.kind, A2aErrorKind::InternalError);
        assert_eq!(mapped.message, "internal error");
        assert_eq!(
            denied(AuthError::Forbidden(
                "missing capability: message:post".into()
            ))
            .kind,
            A2aErrorKind::PermissionDenied
        );
    }
}
