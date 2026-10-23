//! The migration register is the directory.
//!
//! `build.rs` lists `migrations/{postgres,sqlite}/NNNN_<slug>.sql` and
//! generates the register `migrate.rs` applies, so a file on disk cannot be
//! left out. The register was once a hand-maintained `include_str!` list, and
//! a migration very nearly shipped unregistered because an edit to it failed
//! silently. These checks keep the hand list from coming back; the unit tests
//! in `migrate.rs` compare the generated register with the directory.

use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn migrate_rs_has_no_hand_written_register() {
    let runner = read("src/migrate.rs");
    assert!(
        !runner.contains("include_str!(\"../../../migrations"),
        "migrate.rs includes a migration file by hand; build.rs registers them"
    );
    assert!(runner.contains("include!(concat!(env!(\"OUT_DIR\"), \"/migrations.rs\"))"));
}

#[test]
fn the_build_script_skips_down_scripts_and_rejects_bad_names() {
    let build = read("build.rs");
    assert!(build.contains("file.ends_with(\"_down.sql\")"));
    assert!(build.contains("is not NNNN_<slug>.sql"));
    assert!(build.contains("share version"));
}
