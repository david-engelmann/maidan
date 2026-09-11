//! Recipe blueprint store (Cluster 370, Wave 2 #18): create/get/list/delete +
//! the JSON `spec` round-trip. Both backends. Instantiation (`recipe_runs`) is
//! Cluster 370.2.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewRecipe, NewWorkspace, RecipeChild, RecipeParam,
    RecipeSpec,
};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");

    let spec = RecipeSpec {
        params: vec![RecipeParam {
            name: "repo".into(),
            required: true,
            description: Some("target repo".into()),
        }],
        definition_of_done: Some("PR merged".into()),
        retry: None,
        children: vec![
            RecipeChild {
                key: "build".into(),
                title: "build it".into(),
                required_skills: vec!["rust".into()],
                depends_on: vec![],
            },
            RecipeChild {
                key: "review".into(),
                title: "review it".into(),
                required_skills: vec![],
                depends_on: vec!["build".into()],
            },
        ],
    };

    let recipe = store
        .create_recipe(NewRecipe {
            workspace_id: ws.id,
            channel_id: channel.id,
            name: "ship-a-feature".into(),
            spec: spec.clone(),
            created_by: member.id,
        })
        .await
        .expect("create");
    assert_eq!(recipe.name, "ship-a-feature");
    assert_eq!(recipe.channel_id, channel.id);
    // The typed spec round-trips through the JSON column verbatim.
    assert_eq!(recipe.spec, spec);

    let got = store.get_recipe(recipe.id).await.expect("get");
    assert_eq!(got.spec.children.len(), 2);
    assert_eq!(got.spec.definition_of_done.as_deref(), Some("PR merged"));

    // A second recipe; list returns both, newest-first, scoped to the workspace.
    let second = store
        .create_recipe(NewRecipe {
            workspace_id: ws.id,
            channel_id: channel.id,
            name: "nightly".into(),
            spec: RecipeSpec::default(),
            created_by: member.id,
        })
        .await
        .expect("create2");
    let list = store.list_recipes(ws.id).await.expect("list");
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, second.id, "newest first");

    // Delete removes it; a second delete is a no-op (false); get → NotFound.
    assert!(store.delete_recipe(recipe.id).await.expect("delete"));
    assert!(!store.delete_recipe(recipe.id).await.expect("delete again"));
    assert!(store.get_recipe(recipe.id).await.is_err());
    assert_eq!(store.list_recipes(ws.id).await.expect("list").len(), 1);
}

#[tokio::test]
async fn recipe_crud_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn recipe_crud_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}
