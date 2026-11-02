### Rust-only jobs that are not required skip when no Rust changed

- **Changed:** A pull request that touches no Rust (nothing under `crates/`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/` or `ci.yml`) skips loom, coverage (llvm-cov), sqlite-vec and replica routing. None of them is a required check, and every required check still runs; pushes to dev and main run everything.
