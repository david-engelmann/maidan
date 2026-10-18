//! `S3Config` reaches `{:?}` wherever a caller logs or wraps it; the object
//! store's credentials must not come along.

use maidan_artifacts::s3::S3Config;

#[test]
fn debug_formatting_hides_both_s3_keys() {
    let config = S3Config {
        endpoint: "https://s3.eu-west-1.amazonaws.com".into(),
        bucket: "maidan-artifacts".into(),
        region: "eu-west-1".into(),
        access_key: "access-key-example".into(),
        secret_key: "secret-key-S3cr3t".into(),
    };
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("access-key-example") && !shown.contains("secret-key-S3cr3t"),
        "{shown}"
    );
    assert!(
        shown.contains("maidan-artifacts") && shown.contains("eu-west-1"),
        "{shown}"
    );
}
