//! Secrets from mounted files (`<NAME>_FILE`).
//!
//! A secret passed as an environment variable is in the container's config,
//! where `docker inspect`, `kubectl describe` and a compose file show it. Each
//! secret-bearing variable can instead name a file (a Docker or Kubernetes
//! secret mount) in `<NAME>_FILE`. Both server and CLI resolve these once, at
//! the start of `main` before any thread exists, into the plain variable, so
//! every place that reads the plain name keeps working unchanged and the file
//! is read exactly once.

use std::ffi::OsString;
use std::path::PathBuf;

/// Each secret-bearing variable and the variable naming a file that holds it.
pub const SECRET_FILE_ENV: &[(&str, &str)] = &[
    ("DATABASE_URL", "DATABASE_URL_FILE"),
    ("FEDERATION_DECRYPT_KEYS", "FEDERATION_DECRYPT_KEYS_FILE"),
    (
        "FEDERATION_ENCRYPTION_KEY",
        "FEDERATION_ENCRYPTION_KEY_FILE",
    ),
    ("MAIDAN_CONTENT_KEK", "MAIDAN_CONTENT_KEK_FILE"),
    (
        "MAIDAN_CONTENT_KEK_PREVIOUS",
        "MAIDAN_CONTENT_KEK_PREVIOUS_FILE",
    ),
    ("MAIDAN_DB_REPLICA_URL", "MAIDAN_DB_REPLICA_URL_FILE"),
    ("MAIDAN_EMBEDDING_API_KEY", "MAIDAN_EMBEDDING_API_KEY_FILE"),
    (
        "MAIDAN_EXPORT_SIGNING_KEY",
        "MAIDAN_EXPORT_SIGNING_KEY_FILE",
    ),
    ("MAIDAN_GITHUB_TOKEN", "MAIDAN_GITHUB_TOKEN_FILE"),
    (
        "MAIDAN_GITHUB_WEBHOOK_SECRET",
        "MAIDAN_GITHUB_WEBHOOK_SECRET_FILE",
    ),
    (
        "MAIDAN_OIDC_CLIENT_SECRET",
        "MAIDAN_OIDC_CLIENT_SECRET_FILE",
    ),
    (
        "MAIDAN_RATE_LIMIT_REDIS_URL",
        "MAIDAN_RATE_LIMIT_REDIS_URL_FILE",
    ),
    ("MAIDAN_SESSION_SECRET", "MAIDAN_SESSION_SECRET_FILE"),
    ("MAIDAN_SLACK_BOT_TOKEN", "MAIDAN_SLACK_BOT_TOKEN_FILE"),
    (
        "MAIDAN_SLACK_SIGNING_SECRET",
        "MAIDAN_SLACK_SIGNING_SECRET_FILE",
    ),
    ("MAIDAN_SMTP_PASSWORD", "MAIDAN_SMTP_PASSWORD_FILE"),
    (
        "MAIDAN_SUBSCRIBE_RESUME_SECRET",
        "MAIDAN_SUBSCRIBE_RESUME_SECRET_FILE",
    ),
    ("MAIDAN_VAPID_PRIVATE_KEY", "MAIDAN_VAPID_PRIVATE_KEY_FILE"),
    ("S3_ACCESS_KEY_ID", "S3_ACCESS_KEY_ID_FILE"),
    ("S3_SECRET_ACCESS_KEY", "S3_SECRET_ACCESS_KEY_FILE"),
];

/// Why a `_FILE` variable refused boot. No variant carries the secret or any
/// part of the file: the path and the I/O error are all it names.
#[derive(Debug, thiserror::Error)]
pub enum SecretFileError {
    #[error("{name} and {file_var} are both set; set one of them")]
    BothSet {
        name: &'static str,
        file_var: &'static str,
    },
    #[error("{file_var}={}: cannot read the file: {source}", path.display())]
    Unreadable {
        file_var: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{file_var}={}: the file is not UTF-8", path.display())]
    NotUtf8 {
        file_var: &'static str,
        path: PathBuf,
    },
    #[error("{file_var}={}: the file is empty", path.display())]
    Empty {
        file_var: &'static str,
        path: PathBuf,
    },
}

/// The value each `_FILE` variable names, keyed by the plain variable it
/// stands for. `get` reads a variable; a plain variable set to the empty
/// string counts as unset, because compose interpolation (`${X:-}`) writes
/// one wherever the operator set nothing.
pub fn resolve_secret_files(
    get: impl Fn(&str) -> Option<OsString>,
) -> Result<Vec<(&'static str, String)>, SecretFileError> {
    let mut resolved = Vec::new();
    for &(name, file_var) in SECRET_FILE_ENV {
        let Some(path) = get(file_var).filter(|p| !p.is_empty()) else {
            continue;
        };
        if get(name).is_some_and(|v| !v.is_empty()) {
            return Err(SecretFileError::BothSet { name, file_var });
        }
        let path = PathBuf::from(path);
        let bytes = std::fs::read(&path).map_err(|source| SecretFileError::Unreadable {
            file_var,
            path: path.clone(),
            source,
        })?;
        let text = String::from_utf8(bytes).map_err(|_| SecretFileError::NotUtf8 {
            file_var,
            path: path.clone(),
        })?;
        // `echo secret > file` and most editors end the file with a newline,
        // which no secret means to contain.
        let value = text.trim_end_matches(['\n', '\r']);
        if value.is_empty() {
            return Err(SecretFileError::Empty { file_var, path });
        }
        resolved.push((name, value.to_string()));
    }
    Ok(resolved)
}

/// Resolve every `_FILE` variable in this process's environment into its plain
/// variable and return the plain names that were filled.
///
/// # Safety
///
/// No other thread may read or write the process environment while this runs,
/// since it calls `std::env::set_var`. Calling it first thing in `main`, before
/// an async runtime or any other thread starts, meets this.
pub unsafe fn load_secret_files() -> Result<Vec<&'static str>, SecretFileError> {
    let resolved = resolve_secret_files(|name| std::env::var_os(name))?;
    Ok(resolved
        .into_iter()
        .map(|(name, value)| {
            std::env::set_var(name, value);
            name
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::Write;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let map: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn secret_file(contents: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(contents).unwrap();
        file
    }

    #[test]
    fn a_file_alone_supplies_the_variable() {
        let file = secret_file(b"ghp_example");
        let path = file.path().to_str().unwrap();
        let resolved = resolve_secret_files(env(&[("MAIDAN_GITHUB_TOKEN_FILE", path)])).unwrap();
        assert_eq!(
            resolved,
            vec![("MAIDAN_GITHUB_TOKEN", "ghp_example".to_string())]
        );
    }

    #[test]
    fn the_plain_variable_alone_is_left_to_its_readers() {
        let resolved =
            resolve_secret_files(env(&[("MAIDAN_GITHUB_TOKEN", "ghp_example")])).unwrap();
        assert!(resolved.is_empty(), "{resolved:?}");
    }

    #[test]
    fn both_forms_set_refuses_and_names_the_variable() {
        let file = secret_file(b"from-file");
        let path = file.path().to_str().unwrap();
        let err = resolve_secret_files(env(&[
            ("MAIDAN_SESSION_SECRET", "from-env"),
            ("MAIDAN_SESSION_SECRET_FILE", path),
        ]))
        .unwrap_err();
        assert!(matches!(
            err,
            SecretFileError::BothSet {
                name: "MAIDAN_SESSION_SECRET",
                ..
            }
        ));
        let message = err.to_string();
        assert!(message.contains("MAIDAN_SESSION_SECRET_FILE"), "{message}");
        assert!(!message.contains("from-env") && !message.contains("from-file"));
    }

    #[test]
    fn an_empty_plain_variable_does_not_count_as_set() {
        let file = secret_file(b"postgres://maidan@db/maidan\n");
        let path = file.path().to_str().unwrap();
        let resolved =
            resolve_secret_files(env(&[("DATABASE_URL", ""), ("DATABASE_URL_FILE", path)]))
                .unwrap();
        assert_eq!(
            resolved,
            vec![("DATABASE_URL", "postgres://maidan@db/maidan".to_string())]
        );
    }

    #[test]
    fn a_missing_file_refuses_and_names_the_variable_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent");
        let err = resolve_secret_files(env(&[("MAIDAN_CONTENT_KEK_FILE", path.to_str().unwrap())]))
            .unwrap_err();
        assert!(matches!(err, SecretFileError::Unreadable { .. }));
        let message = err.to_string();
        assert!(message.starts_with("MAIDAN_CONTENT_KEK_FILE="), "{message}");
        assert!(message.contains("absent"), "{message}");
    }

    #[test]
    fn the_trailing_newline_is_trimmed_and_nothing_else() {
        let file = secret_file(b"  sec ret  \r\n\n");
        let path = file.path().to_str().unwrap();
        let resolved = resolve_secret_files(env(&[("MAIDAN_SLACK_BOT_TOKEN_FILE", path)])).unwrap();
        assert_eq!(resolved[0].1, "  sec ret  ");
    }

    #[test]
    fn an_empty_or_newline_only_file_refuses() {
        let file = secret_file(b"\n");
        let path = file.path().to_str().unwrap();
        let err =
            resolve_secret_files(env(&[("MAIDAN_SLACK_SIGNING_SECRET_FILE", path)])).unwrap_err();
        assert!(matches!(err, SecretFileError::Empty { .. }));
    }

    #[test]
    fn a_file_that_is_not_utf8_refuses_without_echoing_it() {
        let file = secret_file(&[0xff, 0xfe, b's']);
        let path = file.path().to_str().unwrap();
        let err =
            resolve_secret_files(env(&[("MAIDAN_GITHUB_WEBHOOK_SECRET_FILE", path)])).unwrap_err();
        assert!(matches!(err, SecretFileError::NotUtf8 { .. }));
    }

    #[test]
    fn every_secret_has_a_file_form_named_after_it() {
        for (name, file_var) in SECRET_FILE_ENV {
            assert_eq!(*file_var, format!("{name}_FILE"));
        }
    }
}
