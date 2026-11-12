//! The `McpServer` tests, one file per tool family. They sit inside the
//! crate, not under `tests/`, because they reach `pub(crate)` items such as
//! `McpServer::publish_event` that integration tests cannot see. Fixtures
//! more than one family uses live here; a family's own stay in its file.

use super::*;
use maidan_store::test_support::SetResultClaiming;

use maidan_artifacts::LocalFsStore;
use maidan_auth::AuthContext;
use maidan_search::HashV1Provider;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::*;

// Each family is gated again so the source scans that skip a
// `#[cfg(test)] mod x;` file skip these too.
#[cfg(test)]
mod access;
#[cfg(test)]
mod claims;
#[cfg(test)]
mod events;
#[cfg(test)]
mod members;
#[cfg(test)]
mod protocol;
#[cfg(test)]
mod resources;
#[cfg(test)]
mod results;
#[cfg(test)]
mod reviews;
#[cfg(test)]
mod threads;
#[cfg(test)]
mod waits;
#[cfg(test)]
mod workspace;

/// The evidence root of what the thread was last handed to review with,
/// which an approval names.
async fn handed_root(store: &Arc<dyn Store>, thread_id: ThreadId) -> String {
    store
        .latest_review_packet(thread_id)
        .await
        .unwrap()
        .expect("handed to review")
        .evidence_root
}

async fn mk_server() -> (McpServer, ThreadId, MemberId) {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();

    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let workspace = store
        .create_workspace(NewWorkspace {
            name: "mcp-subscribe".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: workspace.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: workspace.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();

    let server = McpServer::new(
        store,
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(maidan_search::SqliteSearch::new(pool)),
        Arc::new(HashV1Provider),
    );
    (server, thread.id, member.id)
}

fn request(id: i64, method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(id)),
        method: method.into(),
        params,
    }
}

/// A family file `mod.rs` does not declare would never compile, so its tests
/// would vanish without a failure.
#[test]
fn every_family_file_is_declared_as_a_module() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/server/tests");
    let declared = include_str!("mod.rs");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        if stem == "mod" {
            continue;
        }
        assert!(
            declared.contains(&format!("\nmod {stem};\n")),
            "{stem}.rs is not declared in server/tests/mod.rs"
        );
    }
}
