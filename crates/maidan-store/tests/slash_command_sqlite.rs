use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewSlashCommand, NewWorkspace, SlashHandlerKind};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn sqlite_creates_slash_command() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let _member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let command = store
        .create_slash_command(NewSlashCommand {
            workspace_id: ws.id,
            name: "ping".into(),
            description: None,
            handler_kind: SlashHandlerKind::McpTool,
            handler_target: "list_channels".into(),
            secret_ciphertext: String::new(),
        })
        .await
        .unwrap_or_else(|e| panic!("create slash command: {e:?}"));
    assert_eq!(command.name, "ping");
    let by_name = store
        .get_slash_command_by_name(ws.id, "ping")
        .await
        .unwrap();
    assert_eq!(by_name.command.id, command.id);
}
