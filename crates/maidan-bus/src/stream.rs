use std::pin::Pin;

use futures::Stream;
use maidan_types::Event;

/// A pinned, boxed event stream returned by [`crate::EventBus::subscribe`].
///
/// Items are filtered events; subscribers do not see events that fall
/// outside their filter. The stream ends when the publisher drops or the
/// underlying backend reports a permanent error.
pub type EventStream = Pin<Box<dyn Stream<Item = Event> + Send>>;
