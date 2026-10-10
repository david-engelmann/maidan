//! In-query thread read filter tests.

use maidan_search::{Search, SearchFilters};
use maidan_store::prelude::*;
use maidan_types::*;
use std::sync::Arc;

async fn setup() -> (Arc<dyn Store>, maidan_search::SqliteSearch) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    maidan_store::run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search = maidan_search::SqliteSearch::new(pool);
    (store, search)
}

#[tokio::test]
async fn two_tenant_isolation() {
    let (store, search) = setup().await;
    
    // Workspace A
    let ws_a = store.create_workspace(NewWorkspace { name: "a".into() }).await.unwrap();
    let alice = store.create_member(NewMember {
        workspace_id: ws_a.id,
        handle: "alice".into(),
        display_name: None,
        kind: MemberKind::Human,
    }).await.unwrap();
    
    // Workspace B
    let ws_b = store.create_workspace(NewWorkspace { name: "b".into() }).await.unwrap();
    let carol = store.create_member(NewMember {
        workspace_id: ws_b.id,
        handle: "carol".into(),
        display_name: None,
        kind: MemberKind::Human,
    }).await.unwrap();
    
    // Message in A
    let ch_a = store.create_channel(NewChannel {
        workspace_id: ws_a.id,
        name: "general".into(),
        topic: None,
        private: false,
    }).await.unwrap();
    let th_a = store.create_thread(NewThread {
        channel_id: ch_a.id,
        parent_thread_id: None,
        title: None,
        description: None,
    }).await.unwrap();
    store.post_message(NewMessage {
        thread_id: th_a.id,
        author_id: alice.id,
        body: "alpha unique".into(),
        metadata: serde_json::json!({}),
        content: None,
    }).await.unwrap();
    
    // Message in B
    let ch_b = store.create_channel(NewChannel {
        workspace_id: ws_b.id,
        name: "general".into(),
        topic: None,
        private: false,
    }).await.unwrap();
    let th_b = store.create_thread(NewThread {
        channel_id: ch_b.id,
        parent_thread_id: None,
        title: None,
        description: None,
    }).await.unwrap();
    store.post_message(NewMessage {
        thread_id: th_b.id,
        author_id: carol.id,
        body: "alpha unique".into(),
        metadata: serde_json::json!({}),
        content: None,
    }).await.unwrap();
    
    // Alice searches in A
    let filters = SearchFilters { reader_id: Some(alice.id), ..Default::default() };
    let hits = search.search_messages(ws_a.id, "alpha", 10, &filters).await.unwrap();
    
    assert_eq!(hits.len(), 1, "sees only A's message");
    assert_eq!(hits[0].workspace_id, ws_a.id);
}

#[tokio::test]
async fn full_page_with_hidden_rows() {
    let (store, search) = setup().await;
    
    let ws = store.create_workspace(NewWorkspace { name: "w".into() }).await.unwrap();
    let alice = store.create_member(NewMember {
        workspace_id: ws.id,
        handle: "alice".into(),
        display_name: None,
        kind: MemberKind::Human,
    }).await.unwrap();
    let bob = store.create_member(NewMember {
        workspace_id: ws.id,
        handle: "bob".into(),
        display_name: None,
        kind: MemberKind::Human,
    }).await.unwrap();
    
    let public_ch = store.create_channel(NewChannel {
        workspace_id: ws.id,
        name: "public".into(),
        topic: None,
        private: false,
    }).await.unwrap();
    let private_ch = store.create_channel(NewChannel {
        workspace_id: ws.id,
        name: "secret".into(),
        topic: None,
        private: true,
    }).await.unwrap();
    store.add_channel_member(private_ch.id, bob.id, ChannelMemberRole::Member).await.unwrap();
    
    const N: usize = 5;
    for i in 0..N {
        let th = store.create_thread(NewThread {
            channel_id: public_ch.id,
            parent_thread_id: None,
            title: None,
            description: None,
        }).await.unwrap();
        store.post_message(NewMessage {
            thread_id: th.id,
            author_id: alice.id,
            body: format!("visible beta message number {i}"),
            metadata: serde_json::json!({}),
            content: None,
        }).await.unwrap();
    }
    for i in 0..N {
        let th = store.create_thread(NewThread {
            channel_id: private_ch.id,
            parent_thread_id: None,
            title: None,
            description: None,
        }).await.unwrap();
        store.post_message(NewMessage {
            thread_id: th.id,
            author_id: bob.id,
            body: format!("hidden beta message number {i}"),
            metadata: serde_json::json!({}),
            content: None,
        }).await.unwrap();
    }
    
    let filters = SearchFilters { reader_id: Some(alice.id), ..Default::default() };
    let hits = search.search_messages(ws.id, "beta", N as i64, &filters).await.unwrap();
    
    assert_eq!(hits.len(), N, "page of {N} returns {N} visible hits");
}
