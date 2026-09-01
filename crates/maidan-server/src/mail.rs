//! Email/SMTP notification transport (Cluster 247, Program C — Arc I). The first
//! off-platform delivery transport (beyond `deliver_http` for webhooks). It is
//! **config-gated**: [`SmtpConfig::from_env`] returns `None` unless `MAIDAN_SMTP_HOST`
//! and `MAIDAN_SMTP_FROM` are set, so a default deployment builds no mailer and sends
//! nothing. **Wired into the notification router** (Cluster 249): when a per-recipient
//! notification is written, a member with a delivery address on file is emailed
//! best-effort (spawned so a slow SMTP server never blocks routing), with presence-skip
//! (253) and digest-mode (255) gates applied before send, and durable retry/DLQ via the
//! mail outbox (305–306). The [`MailTransport`] trait keeps room for future transports
//! (SMS, push).

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

    /// Wire-path test: the real `lettre` `SmtpTransport` opens a plaintext SMTP
    /// connection and delivers a message end-to-end. The recording-mock e2e proves
    /// the router *calls* `send`; this proves `send` actually speaks SMTP (envelope
    /// + headers + body on the wire), against an in-process minimal SMTP sink — no
    /// Docker, no real MTA (Integration Reality §3.1).
    #[tokio::test]
    async fn smtp_transport_delivers_a_message_over_the_wire() {
        use std::time::Duration;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();

        // A minimal SMTP server: greet, ack the envelope commands, capture DATA.
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let (r, mut w) = sock.split();
            let mut reader = BufReader::new(r);
            let mut line = String::new();
            let mut data = String::new();
            w.write_all(b"220 localhost ESMTP\r\n").await.unwrap();
            loop {
                line.clear();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let cmd = line.trim_end();
                if cmd.starts_with("EHLO") || cmd.starts_with("HELO") {
                    w.write_all(b"250-localhost\r\n250 OK\r\n").await.unwrap();
                } else if cmd.starts_with("MAIL FROM") || cmd.starts_with("RCPT TO") {
                    w.write_all(b"250 OK\r\n").await.unwrap();
                } else if cmd.starts_with("DATA") {
                    w.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                        .await
                        .unwrap();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).await.unwrap() == 0 {
                            break;
                        }
                        if line.trim_end() == "." {
                            break;
                        }
                        data.push_str(&line);
                    }
                    w.write_all(b"250 OK: queued\r\n").await.unwrap();
                } else if cmd.starts_with("QUIT") {
                    w.write_all(b"221 Bye\r\n").await.unwrap();
                    break;
                } else {
                    w.write_all(b"250 OK\r\n").await.unwrap();
                }
            }
            let _ = tx.send(data);
        });

        let cfg = SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            username: None,
            password: None,
            from: "maidan@example.com".into(),
            starttls: false,
        };
        SmtpTransport::from_config(&cfg)
            .unwrap()
            .send(
                "agent@example.com",
                "You were mentioned",
                "hello from maidan",
            )
            .await
            .unwrap();

        let captured = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("sink did not capture the message in time")
            .unwrap();
        assert!(
            captured.contains("Subject: You were mentioned"),
            "subject header on the wire: {captured}"
        );
        assert!(captured.contains("hello from maidan"), "body on the wire");
        assert!(
            captured.contains("agent@example.com"),
            "To header on the wire"
        );
        assert!(
            captured.contains("maidan@example.com"),
            "From header on the wire"
        );
    }
}
