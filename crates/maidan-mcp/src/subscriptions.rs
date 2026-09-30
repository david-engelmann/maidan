//! Who is subscribed to which MCP resource, and who may be told it changed.
//!
//! A subscription belongs to one [`Subscriber`]: the caller that made it
//! ([`Principal`]) in the MCP session it made it in ([`McpSession`]). An update
//! reaches only the listeners of that same subscriber, and only while the
//! subscriber can still read the resource — the server re-checks access at
//! delivery, so a caller removed from a private channel stops hearing about it.
//!
//! Subscriptions were once one process-wide set that every listener drained,
//! then one set per workspace, so a member heard about private threads another
//! member of the same workspace watched. Keying by subscriber is what makes a
//! subscription private to whoever made it.
//!
//! What this module holds lives in one process: a streamable session's
//! subscriptions go when the session is closed or expires, and a stdio
//! caller's go once it has had no open listener for the idle TTL. A stateless
//! caller's subscriptions are not held here. It has no session, so its
//! subscribe and its listener may reach different replicas; the server keeps
//! them in the store ([`maidan_store::McpSubscriptionStore`]) under the
//! caller's [`Principal::key`], and this module only tracks which stateless
//! callers have a listener open in this process, so that this process can
//! deliver to them and keep their subscriptions from lapsing.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures::Stream;
use maidan_auth::AuthContext;
use maidan_types::{ApiTokenId, AppInstallationId, DelegationGrantId, MemberId, WorkspaceId};
use tokio::sync::broadcast;

use crate::protocol::JsonRpcNotification;

/// The most resources one subscriber may watch at once.
pub const MAX_SUBSCRIPTIONS_PER_SUBSCRIBER: usize = 1024;

/// The MCP session a request arrived in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum McpSession {
    /// Stateless HTTP (`POST /mcp`, and `POST /mcp/streamable` from
    /// `2025-03-26` on). There is no protocol session, so the caller's
    /// credential is its session: what it subscribes to arrives on its own
    /// `GET /mcp/notifications` or `GET /mcp/streamable` listener, on
    /// whichever replica that listener is open.
    Stateless,
    /// A `2024-11-05` streamable session, by `Mcp-Session-Id`.
    Streamable(String),
    /// The stdio transport: one process, one caller.
    Stdio,
}

/// The identity a subscription belongs to: everything about a caller except
/// its capabilities. Two credentials of one member are two principals.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Principal {
    /// Auth disabled, or a trusted in-process caller.
    Bypass,
    Caller {
        workspace_id: WorkspaceId,
        member_id: MemberId,
        actor_id: MemberId,
        token_id: Option<ApiTokenId>,
        app_installation_id: Option<AppInstallationId>,
        delegation_grant_id: Option<DelegationGrantId>,
    },
}

impl Principal {
    /// This principal spelled as one string, the key its stateless
    /// subscriptions are stored under. Every field is part of it: two
    /// credentials of one member are two subscribers.
    pub fn key(&self) -> String {
        fn part<T: std::fmt::Display>(id: Option<T>) -> String {
            id.map_or_else(|| "-".to_string(), |id| id.to_string())
        }
        match self {
            Self::Bypass => "bypass".to_string(),
            Self::Caller {
                workspace_id,
                member_id,
                actor_id,
                token_id,
                app_installation_id,
                delegation_grant_id,
            } => format!(
                "{}/{}/{}/{}/{}/{}",
                workspace_id.0,
                member_id.0,
                actor_id.0,
                part(token_id.map(|t| t.0)),
                part(app_installation_id.map(|a| a.0)),
                part(delegation_grant_id.map(|g| g.0)),
            ),
        }
    }

    pub fn of(auth: &AuthContext) -> Self {
        if auth.bypass {
            return Self::Bypass;
        }
        Self::Caller {
            workspace_id: auth.workspace_id,
            member_id: auth.member_id,
            actor_id: auth.actor_id,
            token_id: auth.token_id,
            app_installation_id: auth.app_installation_id,
            delegation_grant_id: auth.delegation_grant_id,
        }
    }
}

/// A caller in a session: the unit a subscription belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Subscriber {
    pub principal: Principal,
    pub session: McpSession,
}

impl Subscriber {
    pub fn new(auth: &AuthContext, session: McpSession) -> Self {
        Self {
            principal: Principal::of(auth),
            session,
        }
    }
}

/// `resources/subscribe` refused because the subscriber already watches
/// [`MAX_SUBSCRIPTIONS_PER_SUBSCRIBER`] resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionLimit;

struct Entry {
    /// The caller's context at its latest subscribe or listen, used to
    /// re-check access at delivery.
    auth: AuthContext,
    uris: HashSet<String>,
    listeners: usize,
    /// When the last listener went away (or the entry was made without one).
    /// `None` while a listener is open.
    idle_since: Option<Instant>,
}

impl Entry {
    fn new(auth: &AuthContext) -> Self {
        Self {
            auth: auth.clone(),
            uris: HashSet::new(),
            listeners: 0,
            idle_since: Some(Instant::now()),
        }
    }
}

#[derive(Default)]
struct Inner {
    subscribers: HashMap<Subscriber, Entry>,
    by_uri: HashMap<String, HashSet<Subscriber>>,
}

impl Inner {
    fn forget_uri(&mut self, subscriber: &Subscriber, uri: &str) {
        if let Some(watchers) = self.by_uri.get_mut(uri) {
            watchers.remove(subscriber);
            if watchers.is_empty() {
                self.by_uri.remove(uri);
            }
        }
    }

    fn remove_subscriber(&mut self, subscriber: &Subscriber) {
        if let Some(entry) = self.subscribers.remove(subscriber) {
            for uri in &entry.uris {
                self.forget_uri(subscriber, uri);
            }
        }
    }

    fn remove_where(&mut self, doomed: impl Fn(&Subscriber, &Entry) -> bool) {
        let gone: Vec<Subscriber> = self
            .subscribers
            .iter()
            .filter(|(subscriber, entry)| doomed(subscriber, entry))
            .map(|(subscriber, _)| subscriber.clone())
            .collect();
        for subscriber in &gone {
            self.remove_subscriber(subscriber);
        }
    }
}

/// Every session-bound subscription this process holds, and every listener
/// open in it.
pub struct ResourceSubscriptions {
    inner: Mutex<Inner>,
    idle_ttl: Duration,
}

impl ResourceSubscriptions {
    pub fn new(idle_ttl: Duration) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            idle_ttl,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn subscribe(
        &self,
        subscriber: Subscriber,
        auth: &AuthContext,
        uri: &str,
    ) -> Result<(), SubscriptionLimit> {
        let mut inner = self.lock();
        let entry = inner
            .subscribers
            .entry(subscriber.clone())
            .or_insert_with(|| Entry::new(auth));
        if !entry.uris.contains(uri) && entry.uris.len() >= MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
            return Err(SubscriptionLimit);
        }
        entry.auth = auth.clone();
        entry.uris.insert(uri.to_string());
        if entry.listeners == 0 {
            entry.idle_since = Some(Instant::now());
        }
        inner
            .by_uri
            .entry(uri.to_string())
            .or_default()
            .insert(subscriber);
        Ok(())
    }

    /// Whether `subscriber` was watching `uri`.
    pub fn unsubscribe(&self, subscriber: &Subscriber, uri: &str) -> bool {
        let mut inner = self.lock();
        let Some(entry) = inner.subscribers.get_mut(subscriber) else {
            return false;
        };
        let removed = entry.uris.remove(uri);
        if entry.uris.is_empty() && entry.listeners == 0 {
            inner.subscribers.remove(subscriber);
        }
        inner.forget_uri(subscriber, uri);
        removed
    }

    /// The subscribers watching `uri`, each with the context to re-check its
    /// access under.
    pub fn watchers(&self, uri: &str) -> Vec<(Subscriber, AuthContext)> {
        let inner = self.lock();
        inner
            .by_uri
            .get(uri)
            .into_iter()
            .flatten()
            .filter_map(|subscriber| {
                inner
                    .subscribers
                    .get(subscriber)
                    .map(|entry| (subscriber.clone(), entry.auth.clone()))
            })
            .collect()
    }

    fn listener_opened(&self, subscriber: &Subscriber, auth: &AuthContext) {
        let mut inner = self.lock();
        let entry = inner
            .subscribers
            .entry(subscriber.clone())
            .or_insert_with(|| Entry::new(auth));
        entry.auth = auth.clone();
        entry.listeners += 1;
        entry.idle_since = None;
    }

    /// The stateless subscribers with a listener open in this process, each
    /// with the context to re-check its access under.
    pub(crate) fn stateless_listeners(&self) -> Vec<(Subscriber, AuthContext)> {
        self.lock()
            .subscribers
            .iter()
            .filter(|(subscriber, entry)| {
                subscriber.session == McpSession::Stateless && entry.listeners > 0
            })
            .map(|(subscriber, entry)| (subscriber.clone(), entry.auth.clone()))
            .collect()
    }

    /// How long a subscription outlives its last listener.
    pub(crate) fn idle_ttl(&self) -> Duration {
        self.idle_ttl
    }

    fn listener_closed(&self, subscriber: &Subscriber) {
        let mut inner = self.lock();
        let Some(entry) = inner.subscribers.get_mut(subscriber) else {
            return;
        };
        entry.listeners = entry.listeners.saturating_sub(1);
        if entry.listeners == 0 {
            if entry.uris.is_empty() {
                inner.subscribers.remove(subscriber);
            } else {
                entry.idle_since = Some(Instant::now());
            }
        }
    }

    /// Drop every subscription made in streamable session `session_id`.
    pub fn end_session(&self, session_id: &str) {
        self.lock().remove_where(|subscriber, _| {
            matches!(&subscriber.session, McpSession::Streamable(id) if id == session_id)
        });
    }

    /// Drop what has outlived its session: a streamable session's
    /// subscriptions once the session is no longer in `open_sessions`, and
    /// anyone else's once it has had no listener for the idle TTL. Returns how
    /// many resources are still watched, summed over subscribers.
    pub fn reap(&self, open_sessions: &HashSet<String>) -> usize {
        let ttl = self.idle_ttl;
        let mut inner = self.lock();
        inner.remove_where(|subscriber, entry| match &subscriber.session {
            McpSession::Streamable(id) => !open_sessions.contains(id),
            McpSession::Stateless | McpSession::Stdio => {
                entry.idle_since.is_some_and(|since| since.elapsed() >= ttl)
            }
        });
        inner.subscribers.values().map(|e| e.uris.len()).sum()
    }

    /// How many resources are watched, summed over subscribers.
    pub fn len(&self) -> usize {
        self.lock().subscribers.values().map(|e| e.uris.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A resource notification addressed to one subscriber.
#[derive(Debug, Clone)]
pub(crate) struct AddressedNotification {
    pub(crate) subscriber: Arc<Subscriber>,
    pub(crate) notification: JsonRpcNotification,
}

/// One open notification stream (an SSE listener, a streamable session's
/// stream, the stdio loop). It yields only what was addressed to its own
/// subscriber, and while it is open that subscriber's subscriptions are kept.
pub struct NotificationListener {
    rx: broadcast::Receiver<AddressedNotification>,
    subscriber: Subscriber,
    subscriptions: Arc<ResourceSubscriptions>,
}

impl NotificationListener {
    pub(crate) fn open(
        rx: broadcast::Receiver<AddressedNotification>,
        subscriber: Subscriber,
        auth: &AuthContext,
        subscriptions: Arc<ResourceSubscriptions>,
    ) -> Self {
        subscriptions.listener_opened(&subscriber, auth);
        Self {
            rx,
            subscriber,
            subscriptions,
        }
    }

    /// The next notification for this listener; `None` once the server is
    /// gone.
    pub async fn recv(&mut self) -> Option<JsonRpcNotification> {
        loop {
            match self.rx.recv().await {
                Ok(addressed) if *addressed.subscriber == self.subscriber => {
                    return Some(addressed.notification)
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "mcp resource-notification listener lagged");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Every notification already waiting for this listener, without waiting
    /// for more.
    pub fn drain(&mut self) -> Vec<JsonRpcNotification> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(addressed) if *addressed.subscriber == self.subscriber => {
                    out.push(addressed.notification)
                }
                Ok(_) => {}
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "mcp resource-notification listener lagged");
                }
                Err(_) => return out,
            }
        }
    }

    pub fn into_stream(self) -> impl Stream<Item = JsonRpcNotification> + Send {
        futures::stream::unfold(self, |mut listener| async move {
            listener.recv().await.map(|n| (n, listener))
        })
    }
}

impl Drop for NotificationListener {
    fn drop(&mut self) {
        self.subscriptions.listener_closed(&self.subscriber);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caller(workspace: WorkspaceId, member: MemberId) -> AuthContext {
        AuthContext::from_token(
            ApiTokenId(uuid::Uuid::new_v4()),
            member,
            workspace,
            vec!["workspace:read".into()],
        )
    }

    const URI: &str = "maidan://threads/00000000-0000-0000-0000-000000000001";

    #[test]
    fn a_subscription_belongs_to_its_caller_and_session_only() {
        let subs = ResourceSubscriptions::new(Duration::from_secs(60));
        let ws = WorkspaceId(uuid::Uuid::new_v4());
        let alice = caller(ws, MemberId(uuid::Uuid::new_v4()));
        let bob = caller(ws, MemberId(uuid::Uuid::new_v4()));
        let alice_here = Subscriber::new(&alice, McpSession::Stdio);
        subs.subscribe(alice_here.clone(), &alice, URI).unwrap();

        let watchers: Vec<Subscriber> = subs.watchers(URI).into_iter().map(|(s, _)| s).collect();
        assert_eq!(watchers, vec![alice_here.clone()]);
        assert!(!watchers.contains(&Subscriber::new(&bob, McpSession::Stdio)));
        assert!(!watchers.contains(&Subscriber::new(&alice, McpSession::Streamable("s".into()))));

        // Bob unsubscribing what he never subscribed to leaves Alice's alone.
        assert!(!subs.unsubscribe(&Subscriber::new(&bob, McpSession::Stdio), URI));
        assert_eq!(subs.len(), 1);
        assert!(subs.unsubscribe(&alice_here, URI));
        assert!(subs.is_empty());
    }

    #[test]
    fn a_subscriber_cannot_watch_without_limit() {
        let subs = ResourceSubscriptions::new(Duration::from_secs(60));
        let auth = caller(
            WorkspaceId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
        );
        let me = Subscriber::new(&auth, McpSession::Stdio);
        for i in 0..MAX_SUBSCRIPTIONS_PER_SUBSCRIBER {
            subs.subscribe(me.clone(), &auth, &format!("maidan://threads/{i}"))
                .unwrap();
        }
        assert_eq!(
            subs.subscribe(me.clone(), &auth, "maidan://threads/one-more"),
            Err(SubscriptionLimit)
        );
        // Re-subscribing to something already watched is not growth.
        assert!(subs.subscribe(me, &auth, "maidan://threads/0").is_ok());
    }

    #[test]
    fn a_session_takes_its_subscriptions_with_it() {
        let subs = ResourceSubscriptions::new(Duration::from_secs(60));
        let auth = caller(
            WorkspaceId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
        );
        let closed = Subscriber::new(&auth, McpSession::Streamable("closed".into()));
        let expired = Subscriber::new(&auth, McpSession::Streamable("expired".into()));
        let open = Subscriber::new(&auth, McpSession::Streamable("open".into()));
        for s in [&closed, &expired, &open] {
            subs.subscribe(s.clone(), &auth, URI).unwrap();
        }
        subs.end_session("closed");
        let still_open: HashSet<String> = ["open".to_string()].into();
        assert_eq!(subs.reap(&still_open), 1);
        let left: Vec<Subscriber> = subs.watchers(URI).into_iter().map(|(s, _)| s).collect();
        assert_eq!(left, vec![open]);
    }

    #[test]
    fn a_stdio_subscription_lasts_while_listened_to_then_idles_out() {
        let subs = Arc::new(ResourceSubscriptions::new(Duration::ZERO));
        let auth = caller(
            WorkspaceId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
        );
        let me = Subscriber::new(&auth, McpSession::Stdio);
        let (tx, _) = broadcast::channel(4);
        let listener = NotificationListener::open(tx.subscribe(), me.clone(), &auth, subs.clone());
        subs.subscribe(me, &auth, URI).unwrap();
        let none = HashSet::new();
        assert_eq!(subs.reap(&none), 1, "an open listener keeps it");
        drop(listener);
        assert_eq!(subs.reap(&none), 0, "idle past the TTL, it goes");
    }

    #[test]
    fn a_principal_key_tells_every_credential_apart() {
        let ws = WorkspaceId(uuid::Uuid::new_v4());
        let member = MemberId(uuid::Uuid::new_v4());
        let auth = caller(ws, member);
        let one = Principal::of(&auth).key();
        assert_eq!(one, Principal::of(&auth.clone()).key());
        // `caller` mints a new token each time.
        let other_token = Principal::of(&caller(ws, member)).key();
        let other_workspace =
            Principal::of(&caller(WorkspaceId(uuid::Uuid::new_v4()), member)).key();
        assert_ne!(one, other_token);
        assert_ne!(one, other_workspace);
        assert!(one.starts_with(&format!("{}/{}/", ws.0, member.0)));
        assert_eq!(Principal::of(&AuthContext::bypass()).key(), "bypass");
    }

    #[test]
    fn only_open_stateless_listeners_are_listed() {
        let subs = Arc::new(ResourceSubscriptions::new(Duration::from_secs(60)));
        let auth = caller(
            WorkspaceId(uuid::Uuid::new_v4()),
            MemberId(uuid::Uuid::new_v4()),
        );
        let (tx, _) = broadcast::channel(4);
        let stateless = Subscriber::new(&auth, McpSession::Stateless);
        let listener =
            NotificationListener::open(tx.subscribe(), stateless.clone(), &auth, subs.clone());
        let _stdio = NotificationListener::open(
            tx.subscribe(),
            Subscriber::new(&auth, McpSession::Stdio),
            &auth,
            subs.clone(),
        );
        let listed: Vec<Subscriber> = subs
            .stateless_listeners()
            .into_iter()
            .map(|(s, _)| s)
            .collect();
        assert_eq!(listed, vec![stateless]);
        drop(listener);
        assert!(subs.stateless_listeners().is_empty());
    }
}
