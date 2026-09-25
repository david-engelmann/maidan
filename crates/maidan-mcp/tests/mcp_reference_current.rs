//! `book/src/mcp-reference.md` is generated from the live tool catalog, and the
//! docs site regenerates it at publish time. The copy in the tree is what a
//! local `mdbook build` shows and what a reader of the repository sees, so it
//! must not drift: regenerate with
//! `cargo run -p maidan-mcp --bin gen-mcp-reference`.

#[test]
fn the_tracked_mcp_reference_matches_the_catalog() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../book/src/mcp-reference.md");
    let tracked = std::fs::read_to_string(&path).unwrap();
    assert!(
        tracked == maidan_mcp::reference::markdown(),
        "book/src/mcp-reference.md is stale; run `cargo run -p maidan-mcp --bin gen-mcp-reference`"
    );
}
