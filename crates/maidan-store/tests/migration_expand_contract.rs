//! The expand/contract policy stays aligned with the migration runner.
//! A change to how versions are applied has to update docs/Migrations.md
//! in the same commit, and the pages that send an operator there.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn policy_names_the_runner_facts() {
    let policy = read("docs/Migrations.md");
    let runner = read("crates/maidan-store/src/migrate.rs");
    let register = read("crates/maidan-store/tests/migration_register.rs");

    assert!(
        runner.contains("const MIGRATION_LOCK_KEY: i64 = 0x6D69_6772"),
        "lock key moved; update the policy with it"
    );
    assert!(runner.contains("SELECT pg_advisory_lock($1)"));
    assert!(runner.contains("SET statement_timeout = 0"));
    assert!(runner.contains("SET lock_timeout = 0"));
    assert!(
        runner.contains("125 and 133 are unused"),
        "the unused version numbers changed"
    );
    assert!(
        register.contains("!n.ends_with(\"_down.sql\")"),
        "down scripts are filtered somewhere else now"
    );

    let flat = policy.split_whitespace().collect::<Vec<_>>().join(" ");
    for fact in [
        "pg_advisory_lock",
        "0x6D69_6772",
        "statement_timeout",
        "lock_timeout",
        "maidan_migrations",
        "one transaction",
        "CREATE INDEX CONCURRENTLY",
        "_down.sql",
        "include_str!",
        "125 and 133 are unused",
        "0135_member_handle_ci",
        "0136_artifact_erase",
        "migrations_stay_in_lockstep",
        "maxUnavailable",
        "F-54",
        "later release",
        "A compatibility shim does not stay past the contract.",
        "Two versions against one SQLite file is not a supported deploy.",
    ] {
        assert!(flat.contains(fact), "policy missing {fact}");
    }

    let apply = runner
        .split("async fn apply_postgres")
        .nth(1)
        .expect("apply_postgres");
    let apply = apply.split("async fn apply_sqlite").next().expect("split");
    let begin = apply.find("pool.begin()").expect("begin");
    let sql = apply.find("raw_sql(sql)").expect("sql");
    let insert = apply.find("INSERT INTO maidan_migrations").expect("insert");
    let commit = apply.find("tx.commit()").expect("commit");
    assert!(begin < sql && sql < insert && insert < commit);

    let sqlite = runner
        .split("async fn apply_sqlite")
        .nth(1)
        .expect("apply_sqlite");
    let begin = sqlite.find("pool.begin()").expect("sqlite begin");
    let sql = sqlite.find("raw_sql(sql)").expect("sqlite sql");
    let insert = sqlite
        .find("INSERT INTO maidan_migrations")
        .expect("sqlite insert");
    let commit = sqlite.find("tx.commit()").expect("sqlite commit");
    assert!(begin < sql && sql < insert && insert < commit);
}

#[test]
fn operators_can_find_the_policy() {
    let pages = [
        "docs/Production.md",
        "docs/Deploy.md",
        "docs/README.md",
        "docs/Decisions.md",
        "docs/Roadmap.md",
        "docs/Open Work.md",
        "CHANGELOG.md",
    ];
    for page in pages {
        let text = read(page);
        assert!(
            text.contains("Migrations.md"),
            "{page} does not link the policy"
        );
    }
    let sync = read("book/sync-docs.sh");
    assert!(
        sync.contains("Migrations.md"),
        "the book rewrites unpublished docs to GitHub; Migrations.md needs that rewrite or the link check fails"
    );
}
