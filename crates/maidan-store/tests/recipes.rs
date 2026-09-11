//! Recipe blueprint store (Cluster 370, Wave 2 #18): create/get/list/delete +
//! the JSON `spec` round-trip. Both backends. Instantiation (`recipe_runs`) is
//! Cluster 370.2.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EventKind, MemberKind, NewChannel, NewMember, NewRecipe, NewWorkspace, RecipeChild,
    RecipeParam, RecipeSpec,
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

    // --- Instantiation (Cluster 370.2): build + review(←build), build needs "rust".
    let (run, evs) = store
        .instantiate_recipe(recipe.id, serde_json::json!({ "repo": "x/y" }), member.id)
        .await
        .expect("instantiate");
    // Parent + two children each emit a ThreadCreated.
    assert_eq!(evs.len(), 3, "parent + 2 children");
    assert!(evs.iter().all(|e| e.kind == EventKind::ThreadCreated));
    assert_eq!(run.recipe_id, recipe.id);
    assert_eq!(
        run.spec_snapshot, recipe.spec,
        "copy-on-fire freezes the spec"
    );

    // The root thread is the parent, titled after the recipe.
    let root = store.get_thread(run.root_thread_id).await.expect("root");
    assert_eq!(root.title.as_deref(), Some("ship-a-feature"));

    // Two children under the parent; the parent depends on both (lands last).
    let children = store
        .child_thread_summaries(run.root_thread_id)
        .await
        .expect("children");
    assert_eq!(children.len(), 2);
    assert_eq!(
        store
            .list_thread_dependencies(run.root_thread_id)
            .await
            .expect("parent deps")
            .len(),
        2,
        "parent depends on every child"
    );

    // Exactly one inter-child edge (review←build); build carries the "rust" skill.
    let mut child_edges = 0;
    let mut skills = 0;
    for c in &children {
        child_edges += store
            .list_thread_dependencies(c.thread.id)
            .await
            .expect("child deps")
            .len();
        skills += store
            .list_thread_required_skills(c.thread.id)
            .await
            .expect("skills")
            .len();
    }
    assert_eq!(child_edges, 1, "review depends on build");
    assert_eq!(skills, 1, "build requires one skill");

    // Run reads round-trip; latest is this run.
    assert_eq!(
        store.get_recipe_run(run.id).await.expect("get run").id,
        run.id
    );
    assert_eq!(
        store
            .latest_recipe_run(recipe.id)
            .await
            .expect("latest")
            .map(|r| r.id),
        Some(run.id)
    );

    // A missing required param is rejected (no threads created).
    assert!(store
        .instantiate_recipe(recipe.id, serde_json::json!({}), member.id)
        .await
        .is_err());

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
