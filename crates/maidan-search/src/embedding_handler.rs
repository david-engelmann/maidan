//! Indexer handler that writes semantic embeddings on `MessagePosted`.

use std::sync::Arc;

use async_trait::async_trait;
use maidan_store::Store;
use maidan_types::{Event, ThreadState};
use tokio::sync::RwLock;
use tracing::warn;

use crate::embedding_provider::EmbeddingProvider;
use crate::indexer::EventHandler;
use crate::Search;
use crate::SearchError;

pub struct EmbeddingHandler {
    store: Arc<dyn Store>,
    search: Arc<dyn Search>,
    provider: Arc<dyn EmbeddingProvider>,
    health_error: Option<Arc<RwLock<Option<String>>>>,
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
            health_error: None,
        }
    }

    pub fn with_health_error_slot(mut self, health_error: Arc<RwLock<Option<String>>>) -> Self {
        self.health_error = Some(health_error);
        self
    }

    async fn set_health_error(&self, msg: String) {
        if let Some(slot) = &self.health_error {
            *slot.write().await = Some(msg);
        }
    }

    async fn clear_health_error(&self) {
        if let Some(slot) = &self.health_error {
            *slot.write().await = None;
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
                self.set_health_error(format!("load thread failed: {err}"))
                    .await;
                warn!(%err, "embedding handler: load thread failed");
                return;
            }
        };
        if thread.tombstoned_at.is_some() || thread.state == ThreadState::Archived {
            return;
        }

        let embedding = match self.provider.embed(&message.body) {
            Ok(v) => v,
            Err(err) => {
                self.set_health_error(format!("embedding generation failed: {err}"))
                    .await;
                warn!(%err, message_id = %message.id, "embedding generation failed");
                return;
            }
        };
        if let Err(err) = self
            .search
            .upsert_embedding(message.id, self.provider.model_name(), &embedding)
            .await
        {
            match err {
                SearchError::Unsupported(_) => {}
                other => {
                    self.set_health_error(format!("embedding upsert failed: {other}"))
                        .await;
                    warn!(%other, message_id = %message.id, "embedding upsert failed");
                }
            }
            return;
        }
        self.clear_health_error().await;
    }
}
