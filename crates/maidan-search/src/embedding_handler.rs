//! Indexer handler that writes semantic embeddings on `MessagePosted`.

use std::sync::Arc;

use async_trait::async_trait;
use maidan_store::Store;
use maidan_types::{Event, ThreadState};
use tracing::warn;

use crate::embedding_provider::EmbeddingProvider;
use crate::indexer::EventHandler;
use crate::Search;
use crate::SearchError;

pub struct EmbeddingHandler {
    store: Arc<dyn Store>,
    search: Arc<dyn Search>,
    provider: Arc<dyn EmbeddingProvider>,
}

impl EmbeddingHandler {
    pub fn new(
        store: Arc<dyn Store>,
        search: Arc<dyn Search>,
        provider: Arc<dyn EmbeddingProvider>,
    ) -> Self {
        Self {
            store,
            search,
            provider,
        }
    }
}

#[async_trait]
impl EventHandler for EmbeddingHandler {
    async fn handle(&self, event: &Event) {
        let Event::MessagePosted {
            thread_id, message, ..
        } = event
        else {
            return;
        };

        let thread = match self.store.get_thread(*thread_id).await {
            Ok(t) => t,
            Err(err) => {
                warn!(%err, "embedding handler: load thread failed");
                return;
            }
        };
        if thread.tombstoned_at.is_some() || thread.state == ThreadState::Archived {
            return;
        }

        let embedding = self.provider.embed(&message.body);
        if let Err(err) = self
            .search
            .upsert_embedding(message.id, self.provider.model_name(), &embedding)
            .await
        {
            match err {
                SearchError::Unsupported(_) => {}
                other => warn!(%other, message_id = %message.id, "embedding upsert failed"),
            }
        }
    }
}
