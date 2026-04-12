use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use lru::LruCache;
use tracing::{debug, warn};

use crate::error::IngestError;

/// Redis key prefix for all dedup entries.
const KEY_PREFIX: &str = "seismosis:dedup:raw:";

/// Fallback in-memory LRU capacity.
///
/// At ~60 bytes per key this is roughly 6 MB. Covers several thousand events —
/// enough to absorb a Redis restart without same-run duplicates leaking through.
const LRU_CAPACITY: usize = 100_000;

/// Redis-backed idempotency guard with an in-memory LRU fallback.
///
/// ## Protocol
///
/// The check and mark operations are intentionally separate so the caller
/// controls exactly when a dedup key is committed:
///
/// ```text
/// 1. is_duplicate(source_id)   → false  (not yet seen)
/// 2. encode(event)             → bytes
/// 3. produce(bytes)            → Ok(())
/// 4. mark_seen(source_id)      ← written ONLY after successful produce
/// ```
///
/// If the produce fails the key is never written and the event will be
/// retried on the next poll cycle. This eliminates the window where a
/// dropped produce would permanently suppress an event.
///
/// ## Failure modes
///
/// - Redis unavailable at startup: falls back to LRU for the session.
/// - Redis fails during `is_duplicate`: assumes *not* duplicate (safe — drops
///   are worse than duplicates for seismic event ingestion).
/// - Redis fails during `mark_seen`: key recorded in LRU only (non-durable).
///   A service restart may re-produce the same event; the downstream
///   `earthquakes.cleaned` pipeline handles idempotency.
///
/// ## Deduplication guarantee: at-least-once, not exactly-once
///
/// `is_duplicate` and `mark_seen` are **not atomic** with respect to each
/// other or to the Kafka produce. This means a duplicate event can reach the
/// topic in two narrow windows:
///
/// 1. **TTL expiry between check and mark.** If the Redis key written by a
///    previous run expires in the gap between `is_duplicate` returning `false`
///    and `mark_seen` setting the new key, a second copy of the event will be
///    produced. This requires the dedup TTL (default 7 days) to expire while
///    the service is in mid-flight on that specific key — extremely unlikely in
///    practice, but not impossible.
///
/// 2. **Redis failure fallback.** When Redis is unreachable, dedup falls back
///    to the in-memory LRU. The LRU is not persisted across restarts, so an
///    event seen in a previous run can pass the check after a crash/restart and
///    be produced again before the LRU warms up.
///
/// In both cases the downstream `earthquakes.cleaned` stage is expected to
/// handle duplicates (idempotent upsert on `source_id`). The Kafka topic also
/// uses an idempotent producer, which prevents broker-level duplicates within
/// a single producer session, but does not protect against the application-
/// level re-delivery described above.
///
/// **Exactly-once** deduplication would require a distributed transaction
/// spanning the Redis write and the Kafka produce (e.g. Kafka transactions +
/// a transactional outbox). That adds significant operational complexity and
/// is out of scope for this service. At-least-once with idempotent consumers
/// is the standard approach for seismic event pipelines.
///
/// ## Deployment note
///
/// The LRU fallback is NOT safe across multiple service replicas — each
/// instance maintains its own independent LRU. If you scale ingestion to >1
/// replica, Redis availability is a hard requirement.
pub struct Deduplicator {
    conn: Option<redis::aio::ConnectionManager>,
    // Mutex (not RwLock): the deduplicator is accessed only by the single poll
    // loop task, so there is no read-concurrency to exploit. Additionally,
    // mark_seen (put) is a write on every produced event, making the write path
    // as hot as the read path. If this is ever shared across concurrent callers,
    // RwLock + peek() for the read path would be worth revisiting.
    lru: Arc<Mutex<LruCache<String, ()>>>,
    ttl_secs: u64,
    redis_healthy: Arc<std::sync::atomic::AtomicBool>,
}

impl Deduplicator {
    /// Attempt to connect to Redis. Degrades gracefully to in-memory LRU on
    /// failure and logs a warning.
    pub async fn new(redis_url: &str, ttl_secs: u64) -> Self {
        let conn = match connect(redis_url).await {
            Ok(c) => {
                tracing::info!(redis_url, "Redis dedup store connected");
                Some(c)
            }
            Err(e) => {
                warn!(
                    error = %e,
                    redis_url,
                    "Cannot connect to Redis — using in-memory LRU dedup fallback. \
                     NOT suitable for multi-instance deployment."
                );
                None
            }
        };

        let has_conn = conn.is_some();
        Self {
            conn,
            lru: Arc::new(Mutex::new(LruCache::new(
                NonZeroUsize::new(LRU_CAPACITY).expect("LRU_CAPACITY is nonzero"),
            ))),
            ttl_secs,
            redis_healthy: Arc::new(std::sync::atomic::AtomicBool::new(has_conn)),
        }
    }

    /// Return `true` if `source_id` has already been processed.
    ///
    /// **Read-only** — does not modify any state. Pair with [`mark_seen`] after
    /// a successful Kafka produce.
    ///
    /// On Redis failure, assumes *not* duplicate so the event is not silently
    /// dropped.
    pub async fn is_duplicate(&self, source_id: &str) -> bool {
        // ── Fast path: in-memory LRU (O(1), no network) ──────────────────
        // `peek` is used intentionally — it does not update LRU recency order,
        // keeping this a pure read.
        {
            let lru = self.lru.lock().expect("LRU mutex should not be poisoned");
            if lru.peek(source_id).is_some() {
                debug!(source_id, "Dedup: LRU hit (duplicate)");
                return true;
            }
        }

        // ── Durable path: Redis EXISTS ────────────────────────────────────
        match &self.conn {
            Some(conn) => {
                let key = format!("{}{}", KEY_PREFIX, source_id);
                let mut c = conn.clone();

                let result: redis::RedisResult<bool> = redis::cmd("EXISTS")
                    .arg(&key)
                    .query_async(&mut c)
                    .await;

                match result {
                    Ok(exists) => {
                        if exists {
                            debug!(source_id, "Dedup: Redis EXISTS hit (duplicate)");
                        }
                        self.set_redis_healthy(true);
                        exists
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            source_id,
                            "Redis EXISTS failed — assuming not duplicate to avoid dropping event"
                        );
                        self.set_redis_healthy(false);
                        false
                    }
                }
            }
            // No Redis connection — LRU-only mode; already checked above.
            None => false,
        }
    }

    /// Mark `source_id` as processed in the dedup store.
    ///
    /// **Must be called only after a successful Kafka produce.** If the produce
    /// failed, do not call this — the absence of the key allows the event to be
    /// retried on the next poll cycle.
    ///
    /// The LRU is always updated. Redis is updated when available; on Redis
    /// failure the key survives only in the LRU (non-durable across restarts).
    pub async fn mark_seen(&self, source_id: &str) {
        // Always update the in-memory LRU (fast, no network).
        {
            let mut lru = self.lru.lock().expect("LRU mutex should not be poisoned");
            lru.put(source_id.to_owned(), ());
        }

        // Write the durable key to Redis.
        if let Some(conn) = &self.conn {
            let key = format!("{}{}", KEY_PREFIX, source_id);
            let mut c = conn.clone();

            // Plain SET EX — no NX flag. We already confirmed the key was absent
            // in `is_duplicate`; using SET instead of SET NX avoids a spurious
            // miss if the TTL expired in the narrow window between check and mark.
            let result: redis::RedisResult<()> = redis::cmd("SET")
                .arg(&key)
                .arg(1u8)
                .arg("EX")
                .arg(self.ttl_secs)
                .query_async(&mut c)
                .await;

            match result {
                Ok(()) => {
                    debug!(source_id, "Dedup: Redis key written");
                    self.set_redis_healthy(true);
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        source_id,
                        "Redis SET failed after produce — dedup key is LRU-only (non-durable). \
                         A service restart may re-produce this event."
                    );
                    self.set_redis_healthy(false);
                }
            }
        }
    }

    /// Returns `true` if the Redis backend is (last known) reachable.
    pub fn is_redis_healthy(&self) -> bool {
        self.redis_healthy
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn set_redis_healthy(&self, healthy: bool) {
        self.redis_healthy
            .store(healthy, std::sync::atomic::Ordering::Relaxed);
    }
}

async fn connect(url: &str) -> Result<redis::aio::ConnectionManager, IngestError> {
    let client = redis::Client::open(url).map_err(|e| IngestError::DedupStore(e.to_string()))?;
    redis::aio::ConnectionManager::new(client)
        .await
        .map_err(|e| IngestError::DedupStore(e.to_string()))
}
