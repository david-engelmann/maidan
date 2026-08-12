//! Shared search assertions. Both backends point at a fully-populated
//! workspace and run the same query battery so their behavior stays in
//! parity.

use maidan_search::{Search, SearchError, SearchFilters, SearchHit};
use maidan_store::Store;
use maidan_types::*;

/// Build a small fixture: one workspace, two channels, three threads,
/// nine messages with words that exercise the search index.
#[allow(dead_code)]
pub async fn seed(store: &dyn Store) -> Fixture {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "search".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let bot = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "bot".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let general = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let release = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "release".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();

    let mut message_ids: Vec<MessageId> = Vec::new();

    let bodies = [
        (general.id, "rust is a systems programming language"),
        (general.id, "tokio powers async rust applications"),
        (general.id, "ferris the unofficial rust mascot"),
        (release.id, "the release is shipping rust changes"),
        (release.id, "no rust this release; pure go"),
        (release.id, "rollback the deployment immediately"),
    ];
    let mut last_thread: Option<(ChannelId, ThreadId)> = None;
    for (channel_id, body) in bodies {
        let thread_id = match last_thread {
            Some((c, t)) if c == channel_id => t,
            _ => {
                let t = store
                    .create_thread(NewThread {
                        channel_id,
                        parent_thread_id: None,
                        title: None,
                    })
                    .await
                    .unwrap();
                last_thread = Some((channel_id, t.id));
                t.id
            }
        };
        let author_id = if body.contains("rust") {
            alice.id
        } else {
            bot.id
        };
        let m = store
            .post_message(NewMessage {
                thread_id,
                author_id,
                body: body.into(),
                metadata: serde_json::json!({"topic": "engineering"}),
                content: None,
            })
            .await
            .unwrap();
        message_ids.push(m.id);
    }

    // tombstone one rust-mentioning message; it should drop out of hits
    let tombstoned = message_ids[0];
    store.tombstone_message(tombstoned).await.unwrap();

    Fixture {
        workspace_id: ws.id,
        general_channel_id: general.id,
        release_channel_id: release.id,
        alice_id: alice.id,
        bot_id: bot.id,
        tombstoned,
        message_ids,
    }
}

#[allow(dead_code)]
pub struct Fixture {
    pub workspace_id: WorkspaceId,
    pub general_channel_id: ChannelId,
    pub release_channel_id: ChannelId,
    pub alice_id: MemberId,
    pub bot_id: MemberId,
    pub tombstoned: MessageId,
    pub message_ids: Vec<MessageId>,
}

#[allow(dead_code)]
pub async fn assert_search_finds_rust(search: &dyn Search, fx: &Fixture) {
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &SearchFilters::default())
        .await
        .expect("search rust");
    // 5 of the 6 messages mention rust, minus 1 tombstoned == 4 hits.
    let bodies: Vec<&str> = hits.iter().map(|h| h.body.as_str()).collect();
    assert_eq!(
        hits.len(),
        4,
        "expected 4 non-tombstoned rust-mentioning messages, got: {bodies:?}"
    );
    assert!(
        !hits.iter().any(|h| h.message_id == fx.tombstoned),
        "tombstoned message must not appear in hits"
    );
    // Every hit's body actually contains 'rust' (case-insensitive).
    for hit in &hits {
        assert!(
            hit.body.to_lowercase().contains("rust"),
            "expected body to contain 'rust', got: {}",
            hit.body
        );
    }
    // Snippets must include the highlight markers when there is a match
    // in the body. (FTS5 may return an unhighlighted snippet for very
    // short results — we accept either.)
    for hit in &hits {
        if !hit.snippet.is_empty() && hit.snippet.contains("rust") {
            assert!(
                hit.snippet.contains("<mark>"),
                "snippet missing mark: {}",
                hit.snippet
            );
        }
    }
}

#[allow(dead_code)]
pub async fn assert_empty_query_rejected(search: &dyn Search, fx: &Fixture) {
    let err = search
        .search_messages(fx.workspace_id, "   ", 10, &SearchFilters::default())
        .await
        .unwrap_err();
    assert!(matches!(err, SearchError::InvalidQuery(_)));
}

#[allow(dead_code)]
pub async fn assert_unknown_term_returns_empty(search: &dyn Search, fx: &Fixture) {
    let hits = search
        .search_messages(
            fx.workspace_id,
            "xyzzy-not-in-corpus",
            10,
            &SearchFilters::default(),
        )
        .await
        .unwrap();
    assert!(hits.is_empty());
}

/// Run all shared assertions in one call so each backend test stays
/// short.
#[allow(dead_code)]
pub async fn run_search_suite(search: &dyn Search, fx: &Fixture) {
    assert_search_finds_rust(search, fx).await;
    assert_empty_query_rejected(search, fx).await;
    assert_unknown_term_returns_empty(search, fx).await;
}

#[allow(dead_code)]
pub fn hit_ids(hits: &[SearchHit]) -> Vec<MessageId> {
    hits.iter().map(|h| h.message_id).collect()
}

#[allow(dead_code)]
pub async fn assert_faceted_search(search: &dyn Search, fx: &Fixture) {
    let release_only = SearchFilters {
        channel_id: Some(fx.release_channel_id),
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &release_only)
        .await
        .unwrap();
    assert_eq!(hits.len(), 2, "release channel has two rust messages");

    let general_only = SearchFilters {
        channel_id: Some(fx.general_channel_id),
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &general_only)
        .await
        .unwrap();
    assert_eq!(
        hits.len(),
        2,
        "general has three rust messages minus one tombstoned"
    );

    let human_only = SearchFilters {
        author_kind: Some(MemberKind::Human),
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &human_only)
        .await
        .unwrap();
    assert_eq!(hits.len(), 4);

    let agent_only = SearchFilters {
        author_kind: Some(MemberKind::Agent),
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &agent_only)
        .await
        .unwrap();
    assert!(hits.is_empty(), "agents did not author rust messages");

    let alice_only = SearchFilters {
        author_id: Some(fx.alice_id),
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &alice_only)
        .await
        .unwrap();
    assert_eq!(hits.len(), 4);

    let bot_only = SearchFilters {
        author_id: Some(fx.bot_id),
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "deployment", 10, &bot_only)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "bot authored one deployment message");
}

/// Cluster 200: the RBAC `deny_channels` pre-filter excludes a channel's hits at
/// the query level (exercised on both backends via `assert_faceted_search`'s
/// callers is not enough — this is its own suite entry).
#[allow(dead_code)]
pub async fn assert_deny_channels_filter(search: &dyn Search, fx: &Fixture) {
    // Baseline: "rust" matches messages in both channels.
    let all = search
        .search_messages(fx.workspace_id, "rust", 10, &SearchFilters::default())
        .await
        .unwrap();
    assert!(
        all.iter().any(|h| h.channel_id == fx.general_channel_id)
            && all.iter().any(|h| h.channel_id == fx.release_channel_id),
        "baseline search spans both channels"
    );

    // Deny `general` → no hit comes from it, and `release` hits survive.
    let deny_general = SearchFilters {
        deny_channels: vec![fx.general_channel_id],
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &deny_general)
        .await
        .unwrap();
    assert!(
        !hits.is_empty() && hits.iter().all(|h| h.channel_id == fx.release_channel_id),
        "denying general leaves only release hits, got {hits:?}"
    );

    // Deny both → the pre-filter empties the result.
    let deny_both = SearchFilters {
        deny_channels: vec![fx.general_channel_id, fx.release_channel_id],
        ..SearchFilters::default()
    };
    let hits = search
        .search_messages(fx.workspace_id, "rust", 10, &deny_both)
        .await
        .unwrap();
    assert!(hits.is_empty(), "denying every channel yields no hits");
}
