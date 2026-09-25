//! The SMTP transport against a real SMTP server.
//!
//! The mail worker's tests use an in-process fake transport, so nothing had
//! ever sent a message through `lettre` to a server that speaks SMTP. This
//! does, with Mailpit, and reads the message back over its API: the recipient,
//! sender, subject and body arrive as sent, and a subject carrying CR/LF cannot
//! smuggle an extra header (a `Bcc:` here) into the message.

use std::time::Duration;

use maidan_server::mail::{MailTransport, SmtpConfig, SmtpTransport};
use serde_json::Value;
use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    GenericImage,
};

#[tokio::test]
async fn a_message_sent_over_smtp_arrives_intact() {
    let container = match GenericImage::new("axllent/mailpit", "v1.27")
        .with_exposed_port(1025.tcp())
        .with_exposed_port(8025.tcp())
        .with_wait_for(WaitFor::message_on_stdout("accessible via"))
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap().to_string();
    let smtp_port = container.get_host_port_ipv4(1025).await.unwrap();
    let api_port = container.get_host_port_ipv4(8025).await.unwrap();

    let transport = SmtpTransport::from_config(&SmtpConfig {
        host: host.clone(),
        port: smtp_port,
        username: None,
        password: None,
        from: "Maidan <no-reply@example.com>".into(),
        starttls: false,
    })
    .unwrap();

    transport
        .send("alice@example.com", "Your digest", "3 unread in #general")
        .await
        .expect("delivered to a real SMTP server");
    transport
        .send(
            "bob@example.com",
            "Hello\r\nBcc: mallory@example.com",
            "second",
        )
        .await
        .expect("a CR/LF subject is still one message");

    let api = format!("http://{host}:{api_port}/api/v1");
    let client = reqwest::Client::new();
    let mut listed = Value::Null;
    for _ in 0..30 {
        listed = client
            .get(format!("{api}/messages"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if listed["messages_count"].as_u64() == Some(2) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let messages = listed["messages"].as_array().expect("messages");
    assert_eq!(messages.len(), 2, "{listed}");

    let find = |to: &str| {
        messages
            .iter()
            .find(|m| m["To"][0]["Address"] == to)
            .unwrap_or_else(|| panic!("no message to {to}: {listed}"))
            .clone()
    };
    let digest = find("alice@example.com");
    assert_eq!(digest["Subject"], "Your digest");
    assert_eq!(digest["From"]["Address"], "no-reply@example.com");
    let full: Value = client
        .get(format!("{api}/message/{}", digest["ID"].as_str().unwrap()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        full["Text"].as_str().unwrap().trim(),
        "3 unread in #general"
    );

    let injected = find("bob@example.com");
    assert!(
        injected["Bcc"].as_array().is_none_or(|b| b.is_empty()),
        "a subject's CR/LF added a Bcc header: {injected}"
    );
    let headers: Value = client
        .get(format!(
            "{api}/message/{}/headers",
            injected["ID"].as_str().unwrap()
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(headers.get("Bcc").is_none(), "{headers}");
}
