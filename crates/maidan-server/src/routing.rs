//! Route registration that checks, at compile time, how a handler's
//! extractors reject a request.
//!
//! [`get`], [`post`], [`put`], [`patch`] and [`delete`] take the place of
//! axum's. They accept a handler only when every argument is [`Checked`]: an
//! extractor whose rejection is an RFC 9457 problem (`crate::extract`) or a
//! protocol's own envelope (SCIM, A2A, declared with `wrap_extractor!`), or one
//! that cannot fail on anything the client sends. A handler that takes axum's
//! `Json`, `Path`, `Query`, `Bytes` or `String` directly, whose rejection is
//! `text/plain`, does not compile. Registering a route with axum's own
//! functions is refused by clippy (`disallowed-methods` in `clippy.toml`).
//! Chain methods on one route with [`MethodRouter::merge`]:
//! `put(set).merge(get(read))`.

use axum::extract::{ConnectInfo, MatchedPath, OriginalUri, RawQuery, Request, State};
use axum::handler::Handler;
use axum::http::{HeaderMap, Method, Uri};
use axum::routing::MethodRouter;
use axum::Extension;

/// A handler argument whose rejection the API controls.
#[diagnostic::on_unimplemented(
    message = "`{Self}` would let axum answer a rejection in text/plain",
    label = "take this through crate::extract (ApiJson, ApiPath, ApiQuery, ApiBytes, ApiText) or a protocol's wrap_extractor!",
    note = "see docs/Decisions.md, \"Request rejections are problems\""
)]
pub trait Checked {}

// Infallible, or failing only on a server-side wiring bug (a missing
// extension or state), never on what the client sent.
impl<T> Checked for State<T> {}
impl<T> Checked for Extension<T> {}
impl<T> Checked for Option<Extension<T>> {}
impl<T> Checked for ConnectInfo<T> {}
impl Checked for HeaderMap {}
impl Checked for Method {}
impl Checked for Uri {}
impl Checked for OriginalUri {}
impl Checked for MatchedPath {}
impl Checked for RawQuery {}
impl Checked for Request {}
// A WebSocket handshake that is not one answers per RFC 6455 (400/426),
// before any API semantics apply.
impl Checked for axum::extract::ws::WebSocketUpgrade {}

/// A handler whose every argument is [`Checked`]. `T` is the argument tuple
/// axum's [`Handler`] is implemented for, with its leading marker.
pub trait CheckedHandler<T> {}

impl<F, Fut> CheckedHandler<((),)> for F where F: FnOnce() -> Fut {}

macro_rules! checked_handler {
    ($($ty:ident),+) => {
        impl<F, Fut, M, $($ty,)+> CheckedHandler<(M, $($ty,)+)> for F
        where
            F: FnOnce($($ty,)+) -> Fut,
            $($ty: Checked,)+
        {
        }
    };
}
checked_handler!(T1);
checked_handler!(T1, T2);
checked_handler!(T1, T2, T3);
checked_handler!(T1, T2, T3, T4);
checked_handler!(T1, T2, T3, T4, T5);
checked_handler!(T1, T2, T3, T4, T5, T6);
checked_handler!(T1, T2, T3, T4, T5, T6, T7);
checked_handler!(T1, T2, T3, T4, T5, T6, T7, T8);
checked_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
checked_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
checked_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
checked_handler!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);

macro_rules! method {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[allow(clippy::disallowed_methods)]
        pub fn $name<H, T, S>(handler: H) -> MethodRouter<S>
        where
            H: Handler<T, S> + CheckedHandler<T>,
            T: 'static,
            S: Clone + Send + Sync + 'static,
        {
            axum::routing::$name(handler)
        }
    };
}
method!(
    /// Route `GET` (and `HEAD`) to a checked handler.
    get
);
method!(
    /// Route `POST` to a checked handler.
    post
);
method!(
    /// Route `PUT` to a checked handler.
    put
);
method!(
    /// Route `PATCH` to a checked handler.
    patch
);
method!(
    /// Route `DELETE` to a checked handler.
    delete
);

/// The router's fallbacks: an unknown path is a 404 problem and a known path
/// asked with the wrong method a 405 problem (axum keeps its `Allow` header),
/// not axum's empty body. They take no input, so nothing can be rejected.
pub trait ProblemFallbacks {
    /// Install both fallbacks.
    #[must_use]
    fn problem_fallbacks(self) -> Self;
}

impl<S: Clone + Send + Sync + 'static> ProblemFallbacks for axum::Router<S> {
    #[allow(clippy::disallowed_methods)]
    fn problem_fallbacks(self) -> Self {
        use crate::error::ApiError;
        self.fallback(|| async { ApiError::NotFound })
            .method_not_allowed_fallback(|| async { ApiError::MethodNotAllowed })
    }
}

/// axum's own extractors, whose rejections are `text/plain`, are not
/// [`Checked`]. Each line fails to compile (ambiguous `some_item`) if its type
/// ever implements it, so a blanket impl cannot let them back in.
const _: fn() = || {
    trait AmbiguousIfChecked<A> {
        fn some_item() {}
    }
    impl<T: ?Sized> AmbiguousIfChecked<()> for T {}
    struct IfChecked;
    impl<T: ?Sized + Checked> AmbiguousIfChecked<IfChecked> for T {}

    let _ = <axum::Json<serde_json::Value> as AmbiguousIfChecked<_>>::some_item;
    let _ = <axum::extract::Path<String> as AmbiguousIfChecked<_>>::some_item;
    let _ = <axum::extract::Query<serde_json::Value> as AmbiguousIfChecked<_>>::some_item;
    let _ = <axum::body::Bytes as AmbiguousIfChecked<_>>::some_item;
    let _ = <String as AmbiguousIfChecked<_>>::some_item;
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{ApiJson, ApiPath};

    fn checked<H: CheckedHandler<T>, T>(_: H) {}

    async fn typed(
        _: State<()>,
        _: ApiPath<uuid::Uuid>,
        _: HeaderMap,
        _: ApiJson<serde_json::Value>,
    ) {
    }

    #[test]
    fn a_handler_of_checked_extractors_is_accepted() {
        checked::<_, (axum::extract::Request, _, _, _, _)>(typed);
        let _: MethodRouter<()> = post(typed);
    }
}
