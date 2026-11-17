//! The stack a tool call takes on a tokio worker, in the unoptimized build
//! the tests and the CI binaries run.

use super::*;

/// A tool call through `handle` fits in a quarter of tokio's default 2 MiB
/// worker stack. The rest is for what sits above it in a debug server (hyper,
/// axum and the MCP transport, about 1 MiB) and the artifact backend below it
/// (S3's client, about 320 KiB more than the local store). When the dispatch
/// held a stack slot for every tool's future, this call needed 720 KiB on
/// aarch64, and a debug server's `upload_artifact` on S3 overflowed its worker
/// and aborted. A call that no longer fits aborts this test's process.
#[test]
fn a_tool_call_fits_in_a_quarter_of_a_worker_stack() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_stack_size(512 * 1024)
        .enable_all()
        .build()
        .unwrap();
    let (server, thread_id, member_id) = runtime.block_on(mk_server());
    let workspace_id = runtime.block_on(async {
        let thread = server.store.get_thread(thread_id).await.unwrap();
        server
            .store
            .get_channel(thread.channel_id)
            .await
            .unwrap()
            .workspace_id
    });
    let auth = admin_auth(member_id, workspace_id);
    let upload = request(
        1,
        "tools/call",
        json!({
            "name": "upload_artifact",
            "arguments": { "kind": "attachment", "content_base64": "aGVsbG8=" },
        }),
    );
    // Spawned, so the call runs on the worker whose stack is under test.
    let response = runtime
        .block_on(runtime.spawn(async move { server.handle(upload, &auth).await }))
        .unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
}
