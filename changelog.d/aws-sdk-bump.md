### The S3 client's TLS and HTTP stack is current

- **Security:** the AWS SDK is bumped (aws-config 1.13, aws-sdk-s3 1.154, aws-smithy-runtime 1.16), which Rust 1.91 held back, and the S3 client is built without the SDK's legacy `rustls` feature. That drops h2 0.3, hyper 0.14, rustls 0.21 and rustls-webpki 0.101 from the root and fuzz lockfiles, and with them the accepted RUSTSEC-2026-0258, 0098, 0099, 0104, 0346, 0349 and 0382 in `deny.toml` and `.config/osv-scanner.toml`.
