//! `MAIDAN_ENV`: production unless an explicit development value is set.

/// The two values `MAIDAN_ENV` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentEnv {
    /// Every development switch is refused.
    Production,
    /// Local, compose, CI and other explicit non-production stacks.
    Development,
}

impl DeploymentEnv {
    /// Parse a `MAIDAN_ENV` value.
    ///
    /// Unset or empty is production. Only `production` and `development` are
    /// accepted; any other value (including `prod` and `Production`) is an
    /// error so a typo cannot silently drop the production refusals.
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim) {
            None | Some("") => Ok(Self::Production),
            Some("production") => Ok(Self::Production),
            Some("development") => Ok(Self::Development),
            Some(other) => Err(format!(
                "unknown value {other:?}; set `production` or `development`"
            )),
        }
    }

    pub fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

/// The process `MAIDAN_ENV`, or an error for an unknown value.
pub fn deployment_env() -> Result<DeploymentEnv, String> {
    DeploymentEnv::parse(std::env::var("MAIDAN_ENV").ok().as_deref())
}

/// Whether this process is in production.
///
/// An unknown `MAIDAN_ENV` is treated as production (fail closed). Startup
/// must still refuse the unknown value through [`deployment_env`].
pub fn is_production() -> bool {
    match deployment_env() {
        Ok(env) => env.is_production(),
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_and_empty_are_production() {
        assert_eq!(DeploymentEnv::parse(None), Ok(DeploymentEnv::Production));
        assert_eq!(
            DeploymentEnv::parse(Some("")),
            Ok(DeploymentEnv::Production)
        );
        assert_eq!(
            DeploymentEnv::parse(Some("  ")),
            Ok(DeploymentEnv::Production)
        );
    }

    #[test]
    fn only_production_and_development_are_accepted() {
        assert_eq!(
            DeploymentEnv::parse(Some("production")),
            Ok(DeploymentEnv::Production)
        );
        assert_eq!(
            DeploymentEnv::parse(Some("development")),
            Ok(DeploymentEnv::Development)
        );
    }

    #[test]
    fn unknown_values_are_refused() {
        for value in ["prod", "Production", "dev", "test", "staging"] {
            let err = DeploymentEnv::parse(Some(value)).expect_err(value);
            assert!(err.contains(value), "{err}");
            assert!(
                err.contains("production") && err.contains("development"),
                "{err}"
            );
        }
    }
}
