//! Email/SMTP notification transport (Cluster 247, Program C — Arc I). The first
//! off-platform delivery transport (beyond `deliver_http` for webhooks). It is
//! **config-gated**: [`SmtpConfig::from_env`] returns `None` unless `MAIDAN_SMTP_HOST`
//! and `MAIDAN_SMTP_FROM` are set, so a default deployment builds no mailer and sends
//! nothing. **Not wired into the notification router yet** — a later cluster delivers
//! a member's notifications by email once they opt in and provide an address. The
//! [`MailTransport`] trait keeps room for future transports (SMS, push).

use async_trait::async_trait;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{message::Mailbox, AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("smtp configuration: {0}")]
    Config(String),
    #[error("smtp send: {0}")]
    Send(String),
}

/// SMTP connection settings, read from `MAIDAN_SMTP_*`.
#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    /// The `From:` address (RFC 5322 mailbox, e.g. `Maidan <no-reply@example.com>`).
    pub from: String,
    /// STARTTLS on the submission port (true, default) vs a plaintext relay (false,
    /// for a local test MTA). Implicit-TLS-only servers are not modeled here yet.
    pub starttls: bool,
}

impl SmtpConfig {
    /// Build from the environment, or `None` when SMTP isn't configured (host + from
    /// address are required; the rest have defaults). This is the config gate: no
    /// config → no email transport → nothing sent.
    pub fn from_env() -> Option<Self> {
        let host = std::env::var("MAIDAN_SMTP_HOST")
            .ok()
            .filter(|s| !s.is_empty())?;
        let from = std::env::var("MAIDAN_SMTP_FROM")
            .ok()
            .filter(|s| !s.is_empty())?;
        let port = std::env::var("MAIDAN_SMTP_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(587);
        let username = std::env::var("MAIDAN_SMTP_USERNAME")
            .ok()
            .filter(|s| !s.is_empty());
        let password = std::env::var("MAIDAN_SMTP_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty());
        // Default STARTTLS on; set MAIDAN_SMTP_STARTTLS=false for a plaintext relay.
        let starttls = std::env::var("MAIDAN_SMTP_STARTTLS")
            .map(|v| !matches!(v.as_str(), "false" | "0"))
            .unwrap_or(true);
        Some(Self {
            host,
            port,
            username,
            password,
            from,
            starttls,
        })
    }
}

/// A transport that delivers a notification off-platform. One impl today
/// ([`SmtpTransport`]); the trait leaves room for SMS/push later.
#[async_trait]
pub trait MailTransport: Send + Sync {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), MailError>;
}

/// A `lettre`-backed async SMTP transport.
pub struct SmtpTransport {
    mailer: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpTransport {
    /// Build a mailer from config. Fails only on a malformed `from` address or an
    /// invalid host — never connects here (lettre pools connections lazily on send).
    pub fn from_config(cfg: &SmtpConfig) -> Result<Self, MailError> {
        let from: Mailbox = cfg
            .from
            .parse()
            .map_err(|e| MailError::Config(format!("from address: {e}")))?;
        let builder = if cfg.starttls {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
                .map_err(|e| MailError::Config(e.to_string()))?
        } else {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&cfg.host)
        };
        let mut builder = builder.port(cfg.port);
        if let (Some(u), Some(p)) = (&cfg.username, &cfg.password) {
            builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
        }
        Ok(Self {
            mailer: builder.build(),
            from,
        })
    }
}

#[async_trait]
impl MailTransport for SmtpTransport {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<(), MailError> {
        let to: Mailbox = to
            .parse()
            .map_err(|e| MailError::Config(format!("to address: {e}")))?;
        let email = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .body(body.to_string())
            .map_err(|e| MailError::Config(e.to_string()))?;
        self.mailer
            .send(email)
            .await
            .map_err(|e| MailError::Send(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smtp_config_builds_from_config() {
        let cfg = SmtpConfig {
            host: "smtp.example.com".into(),
            port: 587,
            username: Some("user".into()),
            password: Some("pass".into()),
            from: "Maidan <no-reply@example.com>".into(),
            starttls: true,
        };
        // Building the mailer succeeds (no connection yet); a valid `from` parses.
        assert!(SmtpTransport::from_config(&cfg).is_ok());
    }

    #[test]
    fn smtp_config_rejects_a_bad_from_address() {
        let cfg = SmtpConfig {
            host: "smtp.example.com".into(),
            port: 587,
            username: None,
            password: None,
            from: "not a mailbox".into(),
            starttls: true,
        };
        assert!(matches!(
            SmtpTransport::from_config(&cfg),
            Err(MailError::Config(_))
        ));
    }
}
