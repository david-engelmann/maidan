### The workspace is on Rust edition 2024

- **Changed:** every workspace crate builds as edition 2024. The edition's lints are fixed outright: nested `if let`s are let-chains, macro fragments are plain `expr`, and environment writes are in `unsafe` blocks, with the reason they hold. Formatting keeps the 2021 style (`style_edition` in `rustfmt.toml`); the 2024 style comes in its own change.
