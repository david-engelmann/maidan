//! `listings/validation/TEMPLATE.md` is the record every listing fills in
//! before it is published or submitted (Open Work Next 14). It must keep
//! every field Next 14 requires, so a submission cannot skip one.

use std::path::PathBuf;

fn template() -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../listings/validation/TEMPLATE.md");
    std::fs::read_to_string(path).expect("listings/validation/TEMPLATE.md")
}

#[test]
fn the_validation_template_keeps_every_next_14_field() {
    let t = template();
    for field in [
        "| Listing |",
        "| Release |",
        "| Commit |",
        "| Server image |",
        "index digest",
        "| Listed surface |",
        "| Result |",
        "## Clean install",
        "succeeds with valid parameters",
        "actionable error with bad parameters",
        "`readOnlyHint`, `destructiveHint`, `openWorldHint`",
        "## Example prompts through the platform's own model",
        "Reviewer token",
        "outlasts the longest expected review wait",
        "compose, trigger or satisfy its own\n  approval",
        "verified human identity",
        "close gate stays server-side and durable",
        "Provider-rendered\napproval UI is untrusted display, the Maidan console is the trust root, and an\napproval whose evidence the human did not see in the console is provisional.",
        "## Still open before this listing ships",
        "re-runs the record before a resubmission",
    ] {
        assert!(t.contains(field), "TEMPLATE.md lacks {field:?}");
    }
}
