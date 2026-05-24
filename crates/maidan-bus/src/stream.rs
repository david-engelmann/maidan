use std::pin::Pin;

use futures::Stream;

use crate::item::BusItem;

/// A pinned, boxed stream returned by [`crate::EventBus::subscribe`].
pub type EventStream = Pin<Box<dyn Stream<Item = BusItem> + Send>>;
