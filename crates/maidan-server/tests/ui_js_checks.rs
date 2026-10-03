//! The board's Node checks, run from the integration suite so CI runs them.
//!
//! Open Work's remaining Playwright row: `helpers.test.mjs` and
//! `tsc --noEmit --checkJs`. The integration job already runs every file in
//! `tests/`, which is how these start without a workflow change.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static/ui")
}

fn run(program: &str, args: &[String], dir: &Path) {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .env("npm_config_update_notifier", "false")
        .env("NO_UPDATE_NOTIFIER", "1")
        .output()
        .unwrap_or_else(|err| panic!("could not run {program}: {err}"));
    if output.status.success() {
        return;
    }
    panic!(
        "{program} {} exited {}\nstdout:\n{}\nstderr:\n{}",
        args.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn helpers_test_mjs_passes() {
    let helpers = ui_dir().join("helpers.test.mjs");
    run(
        "node",
        &["--test".to_string(), helpers.display().to_string()],
        &ui_dir(),
    );
}

/// TypeScript 5.9.3 is the compiler these projects were checked with.
/// `npm exec` keeps that pin out of the repo and out of `node_modules`.
fn tsc(project: &Path) {
    let scratch = std::env::temp_dir().join(format!("maidan-tsc-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("tsc scratch dir");
    let args = [
        "exec",
        "--yes",
        "--package",
        "typescript@5.9.3",
        "--",
        "tsc",
        "--noEmit",
        "--checkJs",
        "-p",
    ]
    .into_iter()
    .map(str::to_string)
    .chain(std::iter::once(project.display().to_string()))
    .collect::<Vec<_>>();
    run("npm", &args, &scratch);
}

#[test]
fn tsc_check_js_covers_the_board_and_the_service_worker() {
    let ui = ui_dir();
    tsc(&ui);
    tsc(&ui.join("tsconfig.sw.json"));
}
