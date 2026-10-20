//! The operator CLI reads `DATABASE_URL` too. Neither its `--help` nor its
//! refusal of an unusable URL may print the password or token in it.

use std::process::{Command, Output};

const PASSWORD: &str = "S3cr3tPw";
const TOKEN: &str = "tok_S3cr3t";

fn maidan(database_url: &str, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_maidan"));
    for (name, _) in std::env::vars() {
        if name.starts_with("MAIDAN_") {
            cmd.env_remove(name);
        }
    }
    cmd.env("DATABASE_URL", database_url)
        .env("MAIDAN_ALLOW_INSECURE_DEV_KEK", "1")
        .args(args)
        .output()
        .expect("spawn maidan")
}

fn shown(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn help_names_database_url_without_printing_its_value() {
    let url = format!("postgres://maidan:{PASSWORD}@db.example/maidan?authToken={TOKEN}");
    let output = maidan(&url, &["init", "--help"]);
    let text = shown(&output);
    assert!(
        text.contains("DATABASE_URL"),
        "help still names the variable: {text}"
    );
    assert!(
        !text.contains(PASSWORD) && !text.contains(TOKEN),
        "help printed the credential: {text}"
    );
}

#[test]
fn an_unknown_prefix_is_refused_without_printing_the_password_or_token() {
    for url in [
        format!("postgress://maidan:{PASSWORD}@db.example/maidan"),
        format!("libsql://maidan-demo.turso.io?authToken={TOKEN}"),
    ] {
        let output = maidan(&url, &["init", "--workspace", "w", "--admin-handle", "a"]);
        assert!(!output.status.success(), "{url} must be refused");
        let text = shown(&output);
        assert!(text.contains("cannot detect dialect"), "{text}");
        assert!(
            !text.contains(PASSWORD) && !text.contains(TOKEN),
            "a credential leaked: {text}"
        );
    }
}
