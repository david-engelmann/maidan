//! Schema parity: after every migration, Postgres and SQLite have the same
//! tables, the same columns, the same nullability, the same unique keys and
//! the same foreign keys, apart from the differences listed below with their
//! reasons.
//!
//! `backend_parity` checks that each migration and module exists for both
//! backends. This checks what the migrations actually built, so a column added
//! to one backend only, a `NOT NULL` or `UNIQUE` on one side, or a foreign key
//! that cascades on Postgres and dangles on SQLite fails here. Types are not
//! compared: the dialects spell them differently by design.

use std::collections::BTreeSet;
use std::time::Duration;

use maidan_store::{run_postgres_migrations, run_sqlite_migrations};
use sqlx::{postgres::PgPoolOptions, sqlite::SqlitePoolOptions, PgPool, Row, SqlitePool};
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// SQLite's full-text index: the FTS5 virtual table, its shadow tables, and the
/// `AUTOINCREMENT` counter table. Postgres keeps the same index as the
/// `maidan_messages.search_vec` column.
fn sqlite_internal(table: &str) -> bool {
    table.starts_with("maidan_messages_fts") || table.starts_with("sqlite_")
}

/// Differences that are known, with the reason each is safe. Entries use the
/// same spelling the test reports, so a new difference names the line to add.
const ALLOWED: &[(&str, &str)] = &[
    (
        "column pg_only maidan_messages.search_vec",
        "Postgres full-text search; SQLite's is the maidan_messages_fts table",
    ),
    (
        "column pg_only maidan_peers.remote_workspace_id NOT NULL",
        "SQLite cannot add NOT NULL to an existing column without a table rebuild (migration 0011); every insert writes it, and the model field is not optional",
    ),
    (
        "column sqlite_only maidan_peers.remote_workspace_id",
        "the other half of the entry above",
    ),
    (
        "fk pg_only maidan_federated_ingest.local_event_id -> maidan_events",
        "SQLite 0009 declared no FK. Event ids are AUTOINCREMENT, so an orphaned ingest row can never match a new event",
    ),
    (
        "fk pg_only maidan_task_schedules.recipe_id -> maidan_recipes",
        "SQLite cannot add an FK to an existing column (migration 0074); the scheduler treats a dangling recipe_id as a plain schedule",
    ),
];

#[derive(Default)]
struct Schema {
    tables: BTreeSet<String>,
    /// `table.column`, with ` NOT NULL` appended when the column refuses NULL.
    columns: BTreeSet<String>,
    /// `table(col,col)` for every full-column unique index, primary keys included.
    unique: BTreeSet<String>,
    /// `table.column -> referenced_table`.
    fks: BTreeSet<String>,
}

async fn postgres_schema(pool: &PgPool) -> Schema {
    let mut schema = Schema::default();
    for row in sqlx::query(
        "SELECT c.table_name::text AS t, c.column_name::text AS c, c.is_nullable = 'NO' AS nn
         FROM information_schema.columns c
         JOIN information_schema.tables t
           ON t.table_name = c.table_name AND t.table_schema = c.table_schema
         WHERE c.table_schema = 'public' AND t.table_type = 'BASE TABLE'",
    )
    .fetch_all(pool)
    .await
    .expect("columns")
    {
        let table: String = row.get("t");
        let column: String = row.get("c");
        let not_null: bool = row.get("nn");
        schema.tables.insert(table.clone());
        schema.columns.insert(column_key(&table, &column, not_null));
    }
    // Partial and expression indexes are left out on both sides: they are
    // spelled differently per dialect and cannot be compared by column list.
    for row in sqlx::query(
        "SELECT t.relname::text AS t,
                array_to_string(ARRAY(
                    SELECT a.attname::text FROM unnest(i.indkey) k
                    JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k
                    ORDER BY a.attname), ',') AS c
         FROM pg_index i
         JOIN pg_class t ON t.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         WHERE n.nspname = 'public' AND i.indisunique
           AND i.indpred IS NULL AND i.indexprs IS NULL",
    )
    .fetch_all(pool)
    .await
    .expect("unique")
    {
        let table: String = row.get("t");
        let cols: String = row.get("c");
        schema.unique.insert(format!("{table}({cols})"));
    }
    for row in sqlx::query(
        "SELECT tc.table_name::text AS t, kcu.column_name::text AS c, ccu.table_name::text AS r
         FROM information_schema.table_constraints tc
         JOIN information_schema.key_column_usage kcu
           ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema
         JOIN information_schema.constraint_column_usage ccu
           ON ccu.constraint_name = tc.constraint_name AND ccu.table_schema = tc.table_schema
         WHERE tc.constraint_type = 'FOREIGN KEY' AND tc.table_schema = 'public'",
    )
    .fetch_all(pool)
    .await
    .expect("fks")
    {
        let table: String = row.get("t");
        let column: String = row.get("c");
        let referenced: String = row.get("r");
        schema
            .fks
            .insert(format!("{table}.{column} -> {referenced}"));
    }
    schema
}

async fn sqlite_schema(pool: &SqlitePool) -> Schema {
    let mut schema = Schema::default();
    let tables: Vec<String> = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'table'")
        .fetch_all(pool)
        .await
        .expect("tables")
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .filter(|table| !sqlite_internal(table))
        .collect();
    for table in tables {
        schema.tables.insert(table.clone());
        let mut primary_key = Vec::new();
        for row in sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
            .fetch_all(pool)
            .await
            .expect("table_info")
        {
            let column: String = row.get("name");
            let pk: i64 = row.get("pk");
            // A non-INTEGER primary key column accepts NULL in SQLite unless it
            // says NOT NULL (a documented quirk). Every id here is written from
            // Rust, so a primary key is compared as NOT NULL, as Postgres has it.
            let not_null = row.get::<i64, _>("notnull") == 1 || pk > 0;
            schema.columns.insert(column_key(&table, &column, not_null));
            if pk > 0 {
                primary_key.push(column);
            }
        }
        if !primary_key.is_empty() {
            primary_key.sort();
            schema
                .unique
                .insert(format!("{table}({})", primary_key.join(",")));
        }
        for index in sqlx::query(&format!("PRAGMA index_list(\"{table}\")"))
            .fetch_all(pool)
            .await
            .expect("index_list")
        {
            if index.get::<i64, _>("unique") != 1 || index.get::<i64, _>("partial") == 1 {
                continue;
            }
            let name: String = index.get("name");
            let mut cols = Vec::new();
            let mut expression = false;
            for col in sqlx::query(&format!("PRAGMA index_info(\"{name}\")"))
                .fetch_all(pool)
                .await
                .expect("index_info")
            {
                match col.get::<Option<String>, _>("name") {
                    Some(col) => cols.push(col),
                    None => expression = true,
                }
            }
            if !expression {
                cols.sort();
                schema.unique.insert(format!("{table}({})", cols.join(",")));
            }
        }
        for fk in sqlx::query(&format!("PRAGMA foreign_key_list(\"{table}\")"))
            .fetch_all(pool)
            .await
            .expect("foreign_key_list")
        {
            let column: String = fk.get("from");
            let referenced: String = fk.get("table");
            schema
                .fks
                .insert(format!("{table}.{column} -> {referenced}"));
        }
    }
    schema
}

fn column_key(table: &str, column: &str, not_null: bool) -> String {
    if not_null {
        format!("{table}.{column} NOT NULL")
    } else {
        format!("{table}.{column}")
    }
}

fn differences(kind: &str, pg: &BTreeSet<String>, sqlite: &BTreeSet<String>) -> Vec<String> {
    let pg_only = pg.difference(sqlite).map(|x| format!("{kind} pg_only {x}"));
    let sqlite_only = sqlite
        .difference(pg)
        .map(|x| format!("{kind} sqlite_only {x}"));
    pg_only.chain(sqlite_only).collect()
}

#[tokio::test]
async fn postgres_and_sqlite_migrations_build_the_same_schema() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping schema_parity: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let pg = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&format!(
            "postgres://postgres:postgres@{host}:{port}/postgres"
        ))
        .await
        .expect("connect pg");
    run_postgres_migrations(&pg).await.expect("migrate pg");
    let sqlite = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    run_sqlite_migrations(&sqlite)
        .await
        .expect("migrate sqlite");

    let pg = postgres_schema(&pg).await;
    let lite = sqlite_schema(&sqlite).await;
    assert!(
        pg.tables.len() > 90,
        "postgres introspection found too little"
    );
    assert!(
        lite.tables.len() > 90,
        "sqlite introspection found too little"
    );

    let mut found = differences("table", &pg.tables, &lite.tables);
    let common: BTreeSet<_> = pg.tables.intersection(&lite.tables).cloned().collect();
    let in_common = |key: &String| common.iter().any(|t| key.starts_with(&format!("{t}.")));
    let (pg_cols, lite_cols): (BTreeSet<_>, BTreeSet<_>) = (
        pg.columns
            .iter()
            .filter(|c| in_common(c))
            .cloned()
            .collect(),
        lite.columns
            .iter()
            .filter(|c| in_common(c))
            .cloned()
            .collect(),
    );
    found.extend(differences("column", &pg_cols, &lite_cols));
    found.extend(differences("unique", &pg.unique, &lite.unique));
    found.extend(differences("fk", &pg.fks, &lite.fks));

    let allowed: BTreeSet<&str> = ALLOWED.iter().map(|(entry, _)| *entry).collect();
    let unexpected: Vec<&String> = found
        .iter()
        .filter(|x| !allowed.contains(x.as_str()))
        .collect();
    let stale: Vec<&&str> = allowed
        .iter()
        .filter(|entry| !found.iter().any(|x| x == **entry))
        .collect();
    assert!(
        unexpected.is_empty(),
        "the backends' schemas differ; match them, or add each line to ALLOWED with a reason:\n{}",
        unexpected
            .iter()
            .map(|x| format!("  {x}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        stale.is_empty(),
        "ALLOWED lists differences that no longer exist; remove them: {stale:?}"
    );
}
