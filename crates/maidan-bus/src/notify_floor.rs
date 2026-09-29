//! The self-healing floor under the Postgres listener.
//!
//! A NOTIFY is a pointer to an event id. The listener keeps a high-water
//! mark (`last_seen`) and, when a pointer jumps above it, back-fills the ids
//! in between from the log before hydrating the pointer's own event, so a
//! lost NOTIFY heals on the next one. A pointer below the mark (an id that
//! committed late) is hydrated on its own. A reconnect drains everything
//! above the mark, since NOTIFYs sent while disconnected are gone.
//!
//! The mark only moves past what was delivered: when a back-fill stops on a
//! store error, or the pointer's own event cannot be read, it stays below
//! the missing ids so the next pointer or reconnect drains them again (and
//! may deliver the pointer's event a second time; subscribers dedupe by
//! `log_id`). The
//! one gap it cannot see is an id at or below the mark whose NOTIFY never
//! arrived (it committed late and the notification was lost); the durable
//! consumers' log reconcile covers that.
//!
//! The logic reads the log through [`EventLog`], so the simulation in this
//! module can run it against a model log with faults.

use std::sync::Arc;

use async_trait::async_trait;
use maidan_types::{BusEnvelope, ContentKeyring};
use sqlx::PgPool;

use crate::error::BusError;
use crate::hydrate_stats::{HydrateResult, HydrateStats};
use crate::sharded::ShardedBroadcast;

/// Page size for the back-fill: a gap or reconnect drains the missed range
/// in batches of this size, so even a large gap (a long `LISTEN` disconnect)
/// heals without loading it all at once.
pub(crate) const BACKFILL_BATCH: i64 = 256;

/// The event log as the listener reads it.
#[async_trait]
pub(crate) trait EventLog: Send + Sync {
    /// The newest event id, 0 for an empty log.
    async fn head(&self) -> Result<i64, BusError>;
    /// Up to `limit` committed events with `id > after`, in id order. An
    /// event that cannot become an envelope is an `Err` beside its id.
    async fn page_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<(i64, Result<BusEnvelope, BusError>)>, BusError>;
    /// The event with this id.
    async fn get(&self, id: i64) -> Result<BusEnvelope, BusError>;
}

/// `maidan_events`, with sealed words opened by the store's keyring.
pub(crate) struct PgEventLog {
    pub(crate) pool: PgPool,
    pub(crate) keys: Arc<ContentKeyring>,
}

fn envelope_from_stored(stored: &maidan_types::StoredEvent) -> Result<BusEnvelope, BusError> {
    BusEnvelope::from_stored(stored).map_err(|err| BusError::HydrateFailed {
        log_id: stored.id,
        reason: err.to_string(),
    })
}

#[async_trait]
impl EventLog for PgEventLog {
    async fn head(&self) -> Result<i64, BusError> {
        Ok(maidan_store::postgres::events::max_event_id(&self.pool).await?)
    }

    async fn page_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<(i64, Result<BusEnvelope, BusError>)>, BusError> {
        let page = maidan_store::postgres::events::list_after_global_each(
            &self.pool, &self.keys, after, limit,
        )
        .await?;
        Ok(page
            .into_iter()
            .map(|(id, stored)| {
                let envelope = match stored {
                    Ok(stored) => envelope_from_stored(&stored),
                    Err(err) => Err(BusError::HydrateFailed {
                        log_id: id,
                        reason: err.to_string(),
                    }),
                };
                (id, envelope)
            })
            .collect())
    }

    async fn get(&self, id: i64) -> Result<BusEnvelope, BusError> {
        let stored = maidan_store::postgres::events::get_by_id(&self.pool, &self.keys, id)
            .await
            .map_err(|err| match err {
                maidan_store::StoreError::NotFound => BusError::HydrateNotFound { log_id: id },
                maidan_store::StoreError::Database(e) => BusError::Database(e),
                other => BusError::HydrateFailed {
                    log_id: id,
                    reason: other.to_string(),
                },
            })?;
        envelope_from_stored(&stored)
    }
}

/// How far a drain got. `complete` is false when a store error stopped it
/// before the end of the range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Drained {
    pub(crate) reached: i64,
    pub(crate) complete: bool,
}

/// Publish every event with `from_exclusive < id < to_exclusive` (no upper
/// bound when `None`) onto `tx`, in id order and in bounded batches.
/// `reached` is the last id delivered, or `from_exclusive`.
pub(crate) async fn drain<L: EventLog + ?Sized>(
    log: &L,
    tx: &ShardedBroadcast,
    stats: &HydrateStats,
    from_exclusive: i64,
    to_exclusive: Option<i64>,
) -> Drained {
    let mut cursor = from_exclusive;
    loop {
        let batch = match log.page_after(cursor, BACKFILL_BATCH).await {
            Ok(batch) => batch,
            Err(err) => {
                tracing::warn!(error = %err, after = cursor, "notify floor: back-fill query failed");
                return Drained {
                    reached: cursor,
                    complete: false,
                };
            }
        };
        let full = batch.len() as i64 >= BACKFILL_BATCH;
        for (id, envelope) in batch {
            // The pointer's own id (the upper bound) is hydrated by the caller.
            if to_exclusive.is_some_and(|to| id >= to) {
                return Drained {
                    reached: cursor,
                    complete: true,
                };
            }
            match envelope {
                Ok(envelope) => {
                    stats.record(HydrateResult::Backfilled);
                    tx.publish(envelope);
                }
                Err(err) => {
                    stats.record(HydrateResult::Failed);
                    tracing::warn!(error = %err, log_id = id, "notify floor: skip undecodable event");
                }
            }
            cursor = id;
        }
        if !full {
            return Drained {
                reached: cursor,
                complete: true,
            };
        }
    }
}

/// The listener's high-water mark and what it does on each input.
pub(crate) struct NotifyFloor<L> {
    log: L,
    tx: Arc<ShardedBroadcast>,
    stats: Arc<HydrateStats>,
    last_seen: i64,
}

impl<L: EventLog> NotifyFloor<L> {
    /// Start at the log's current head, so only events appended from now on
    /// are back-filled. Fails if the head cannot be read: a guessed mark
    /// would either skip events or replay the whole log on the next drain.
    pub(crate) async fn start(
        log: L,
        tx: Arc<ShardedBroadcast>,
        stats: Arc<HydrateStats>,
    ) -> Result<Self, BusError> {
        let last_seen = log.head().await?;
        Ok(Self {
            log,
            tx,
            stats,
            last_seen,
        })
    }

    #[cfg(all(test, not(feature = "loom")))]
    pub(crate) fn last_seen(&self) -> i64 {
        self.last_seen
    }

    /// A NOTIFY pointing at `log_id`.
    pub(crate) async fn on_pointer(&mut self, log_id: i64) {
        // Everything up to `covered` was delivered or has its own NOTIFY to
        // come (an id that had not committed yet).
        let covered = if log_id > self.last_seen + 1 {
            let drained = drain(
                &self.log,
                &self.tx,
                &self.stats,
                self.last_seen,
                Some(log_id),
            )
            .await;
            if drained.complete {
                log_id - 1
            } else {
                drained.reached
            }
        } else {
            self.last_seen
        };
        // Always hydrate the pointer's own id, never skipping one at or below
        // the mark: it committed late and has not been delivered.
        let delivered = match self.log.get(log_id).await {
            Ok(envelope) => {
                self.stats.record(HydrateResult::Ok);
                self.tx.publish(envelope);
                true
            }
            Err(err) => {
                match err {
                    BusError::HydrateNotFound { .. } => self.stats.record(HydrateResult::NotFound),
                    _ => self.stats.record(HydrateResult::Failed),
                }
                tracing::warn!(error = %err, log_id, "notify pointer not delivered");
                false
            }
        };
        let reached = if delivered && covered >= log_id - 1 {
            covered.max(log_id)
        } else {
            covered
        };
        self.last_seen = self.last_seen.max(reached);
    }

    /// An event that came inline in its NOTIFY (no log row to read).
    pub(crate) fn publish(&self, envelope: BusEnvelope) {
        self.tx.publish(envelope);
    }

    /// The listener reconnected: drain everything above the mark.
    pub(crate) async fn on_reconnect(&mut self) {
        let drained = drain(&self.log, &self.tx, &self.stats, self.last_seen, None).await;
        self.last_seen = drained.reached;
    }
}

/// A seeded simulation of the floor on several replicas against a model log.
///
/// Transactions take ids in order and commit or roll back in any order. A
/// commit notifies every connected replica, in commit order; a NOTIFY can be
/// lost. Replicas disconnect and reconnect, and any log read can fail. At the
/// end every open transaction resolves, disconnected replicas reconnect, and
/// one last event is committed and notified with no faults.
///
/// Checked: a replica only ever delivers committed events, and every
/// committed event reaches every replica, unless its NOTIFY to that replica
/// was lost or failed while its id was already at or below that replica's
/// mark when it committed (the gap the module doc names).
///
/// `MAIDAN_SIM_SEEDS` sets how many seeds run (default 400);
/// `MAIDAN_SIM_SEED` runs one seed, to replay a failure.
#[cfg(all(test, not(feature = "loom")))]
mod sim {
    use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
    use std::sync::Mutex;

    use chrono::{DateTime, Utc};
    use futures::executor::block_on;
    use maidan_types::{Event, EventFilter, Workspace, WorkspaceId};
    use tokio::sync::broadcast;
    use uuid::Uuid;

    use super::*;

    const STEPS: usize = 300;
    const REPLICAS: usize = 3;

    /// splitmix64: small, and the same on every platform.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }

        fn chance(&mut self, per_mille: u64) -> bool {
            self.below(1000) < per_mille
        }
    }

    fn envelope(log_id: i64) -> BusEnvelope {
        let at = DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_default();
        BusEnvelope {
            log_id,
            event: Event::WorkspaceCreated {
                occurred_at: at,
                workspace: Workspace {
                    id: WorkspaceId(Uuid::nil()),
                    name: "w".into(),
                    created_at: at,
                    updated_at: at,
                    tombstoned_at: None,
                },
            },
            attribution: None,
        }
    }

    /// The committed rows, and a fault source for reads.
    struct Log {
        committed: BTreeSet<i64>,
        faults: Rng,
        /// Chance, per mille, that a read fails.
        fail: u64,
    }

    #[derive(Clone)]
    struct SimLog(Arc<Mutex<Log>>);

    impl SimLog {
        fn read(&self) -> Result<std::sync::MutexGuard<'_, Log>, BusError> {
            let mut log = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let fail = log.fail;
            if log.faults.chance(fail) {
                return Err(BusError::Closed);
            }
            Ok(log)
        }
    }

    #[async_trait]
    impl EventLog for SimLog {
        async fn head(&self) -> Result<i64, BusError> {
            Ok(self.read()?.committed.last().copied().unwrap_or(0))
        }

        async fn page_after(
            &self,
            after: i64,
            limit: i64,
        ) -> Result<Vec<(i64, Result<BusEnvelope, BusError>)>, BusError> {
            let log = self.read()?;
            Ok(log
                .committed
                .range(after + 1..)
                .take(usize::try_from(limit).unwrap_or(usize::MAX))
                .map(|&id| (id, Ok(envelope(id))))
                .collect())
        }

        async fn get(&self, id: i64) -> Result<BusEnvelope, BusError> {
            let log = self.read()?;
            if log.committed.contains(&id) {
                Ok(envelope(id))
            } else {
                Err(BusError::HydrateNotFound { log_id: id })
            }
        }
    }

    struct Replica {
        floor: NotifyFloor<SimLog>,
        rx: broadcast::Receiver<BusEnvelope>,
        connected: bool,
        queue: VecDeque<i64>,
        delivered: Vec<i64>,
        /// Events whose NOTIFY to this replica was lost or not delivered.
        missed_notify: HashSet<i64>,
        /// The replica's mark when each event committed.
        mark_at_commit: HashMap<i64, i64>,
    }

    impl Replica {
        fn collect(&mut self) -> Vec<i64> {
            let got: Vec<i64> =
                std::iter::from_fn(|| self.rx.try_recv().ok().map(|e| e.log_id)).collect();
            self.delivered.extend(&got);
            got
        }
    }

    struct World {
        rng: Rng,
        log: SimLog,
        next_id: i64,
        open: Vec<i64>,
        replicas: Vec<Replica>,
        trace: Vec<String>,
    }

    impl World {
        fn new(seed: u64) -> Self {
            let mut rng = Rng(seed);
            let fail = [0, 50, 200][rng.below(3) as usize];
            // Faults start once every replica is up: a failed head read
            // fails the connect (see `start_fails_without_the_head`).
            let log = SimLog(Arc::new(Mutex::new(Log {
                committed: BTreeSet::new(),
                faults: Rng(seed ^ 0x5EED),
                fail: 0,
            })));
            let replicas = (0..REPLICAS)
                .map(|_| {
                    let tx = Arc::new(ShardedBroadcast::new(1 << 16));
                    let rx = tx.subscribe(&EventFilter::default());
                    let floor = block_on(NotifyFloor::start(
                        log.clone(),
                        tx,
                        Arc::new(HydrateStats::default()),
                    ))
                    .unwrap_or_else(|err| panic!("start with no faults: {err}"));
                    Replica {
                        floor,
                        rx,
                        connected: true,
                        queue: VecDeque::new(),
                        delivered: Vec::new(),
                        missed_notify: HashSet::new(),
                        mark_at_commit: HashMap::new(),
                    }
                })
                .collect();
            log.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fail = fail;
            Self {
                rng,
                log,
                next_id: 1,
                open: Vec::new(),
                replicas,
                trace: vec![format!("read failures {fail}/1000")],
            }
        }

        fn stop_faults(&self) {
            self.log
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .fail = 0;
        }

        fn committed(&self) -> BTreeSet<i64> {
            self.log
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .committed
                .clone()
        }

        fn begin(&mut self) {
            self.open.push(self.next_id);
            self.trace.push(format!("begin {}", self.next_id));
            self.next_id += 1;
        }

        fn commit(&mut self, at: usize, lose_notifies: bool) {
            let id = self.open.swap_remove(at);
            self.log
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .committed
                .insert(id);
            let mut lost = Vec::new();
            for (r, replica) in self.replicas.iter_mut().enumerate() {
                replica.mark_at_commit.insert(id, replica.floor.last_seen());
                if replica.connected && !(lose_notifies && self.rng.chance(150)) {
                    replica.queue.push_back(id);
                } else {
                    replica.missed_notify.insert(id);
                    lost.push(r);
                }
            }
            self.trace
                .push(format!("commit {id}, notify lost to {lost:?}"));
        }

        fn deliver(&mut self, r: usize) {
            let Some(id) = self.replicas[r].queue.pop_front() else {
                return;
            };
            let replica = &mut self.replicas[r];
            block_on(replica.floor.on_pointer(id));
            let got = replica.collect();
            if !got.contains(&id) {
                replica.missed_notify.insert(id);
            }
            self.trace.push(format!(
                "replica {r}: notify {id}, delivered {got:?}, mark {}",
                replica.floor.last_seen()
            ));
        }

        fn reconnect(&mut self, r: usize) {
            let replica = &mut self.replicas[r];
            replica.connected = true;
            block_on(replica.floor.on_reconnect());
            let got = replica.collect();
            self.trace.push(format!(
                "replica {r}: reconnect, delivered {got:?}, mark {}",
                replica.floor.last_seen()
            ));
        }

        fn step(&mut self) {
            match self.rng.below(100) {
                0..=29 => self.begin(),
                30..=54 if !self.open.is_empty() => {
                    let at = self.rng.below(self.open.len() as u64) as usize;
                    self.commit(at, true);
                }
                55..=59 if !self.open.is_empty() => {
                    let at = self.rng.below(self.open.len() as u64) as usize;
                    let id = self.open.swap_remove(at);
                    self.trace.push(format!("roll back {id}"));
                }
                60..=91 => {
                    let r = self.rng.below(REPLICAS as u64) as usize;
                    self.deliver(r);
                }
                92..=95 => {
                    let r = self.rng.below(REPLICAS as u64) as usize;
                    let replica = &mut self.replicas[r];
                    if replica.connected {
                        replica.connected = false;
                        let dropped: Vec<i64> = replica.queue.drain(..).collect();
                        replica.missed_notify.extend(&dropped);
                        self.trace
                            .push(format!("replica {r}: disconnect, lost {dropped:?}"));
                    }
                }
                96..=99 => {
                    let r = self.rng.below(REPLICAS as u64) as usize;
                    if !self.replicas[r].connected {
                        self.reconnect(r);
                    }
                }
                _ => {}
            }
            self.check_delivered_are_committed();
        }

        fn check_delivered_are_committed(&self) {
            let committed = self.committed();
            for (r, replica) in self.replicas.iter().enumerate() {
                if let Some(id) = replica.delivered.iter().find(|id| !committed.contains(id)) {
                    self.fail(&format!(
                        "replica {r} delivered {id}, which never committed"
                    ));
                }
            }
        }

        /// Resolve everything, then commit one last event with no faults.
        fn settle(&mut self) {
            self.stop_faults();
            while !self.open.is_empty() {
                self.commit(0, false);
            }
            for r in 0..REPLICAS {
                if !self.replicas[r].connected {
                    self.reconnect(r);
                }
            }
            self.begin();
            self.commit(0, false);
            for r in 0..REPLICAS {
                while !self.replicas[r].queue.is_empty() {
                    self.deliver(r);
                }
            }
        }

        fn check_everything_arrived(&self) {
            let committed = self.committed();
            for (r, replica) in self.replicas.iter().enumerate() {
                let delivered: BTreeSet<i64> = replica.delivered.iter().copied().collect();
                for id in committed.difference(&delivered) {
                    let mark = replica.mark_at_commit.get(id).copied().unwrap_or(0);
                    let explained = replica.missed_notify.contains(id) && *id <= mark;
                    if !explained {
                        self.fail(&format!(
                            "replica {r} never delivered {id} (mark at commit {mark}, notify {})",
                            if replica.missed_notify.contains(id) {
                                "lost or not delivered"
                            } else {
                                "delivered"
                            }
                        ));
                    }
                }
            }
        }

        /// Fail with the last steps, or the whole run when one seed is
        /// replayed.
        fn fail(&self, what: &str) -> ! {
            let keep = if std::env::var("MAIDAN_SIM_SEED").is_ok() {
                self.trace.len()
            } else {
                40
            };
            let steps = &self.trace[self.trace.len().saturating_sub(keep)..];
            panic!("{what}\nsteps:\n{}", steps.join("\n"));
        }
    }

    fn run(seed: u64) {
        let mut world = World::new(seed);
        for _ in 0..STEPS {
            world.step();
        }
        world.settle();
        world.check_delivered_are_committed();
        world.check_everything_arrived();
    }

    fn seeds() -> Vec<u64> {
        if let Some(seed) = std::env::var("MAIDAN_SIM_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            return vec![seed];
        }
        let n = std::env::var("MAIDAN_SIM_SEEDS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(400);
        (0..n).collect()
    }

    #[test]
    fn the_floor_delivers_every_committed_event_under_faults() {
        for seed in seeds() {
            if let Err(panic) = std::panic::catch_unwind(|| run(seed)) {
                let msg = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .unwrap_or_else(|| "panic".into());
                panic!("seed {seed} (replay with MAIDAN_SIM_SEED={seed}): {msg}");
            }
        }
    }

    /// A floor never guesses its starting mark.
    #[test]
    fn start_fails_without_the_head() {
        let log = SimLog(Arc::new(Mutex::new(Log {
            committed: BTreeSet::from([1, 2, 3]),
            faults: Rng(0),
            fail: 1000,
        })));
        let started = block_on(NotifyFloor::start(
            log,
            Arc::new(ShardedBroadcast::new(16)),
            Arc::new(HydrateStats::default()),
        ));
        assert!(started.is_err());
    }

    /// The same seed gives the same run.
    #[test]
    fn a_seed_replays_exactly() {
        let trace = |seed| {
            let mut world = World::new(seed);
            for _ in 0..STEPS {
                world.step();
            }
            world.trace
        };
        assert_eq!(trace(7), trace(7));
    }
}
