use redis::{aio::ConnectionManager, AsyncCommands};
use serde::{de::DeserializeOwned, Serialize};
use tracing::warn;

/// Helper wrapping `redis::aio::ConnectionManager` for typed JSON cache operations.
///
/// All methods return `Option` or `bool` rather than `Result` — cache failures
/// are logged as warnings and callers fall back to the DB. The service degrades
/// gracefully when Redis is unavailable.
#[derive(Clone)]
pub struct Cache {
    conn: ConnectionManager,
}

impl Cache {
    pub fn new(conn: ConnectionManager) -> Self {
        Self { conn }
    }

    /// Fetch and deserialise a cached value. Returns `None` on miss or error.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let mut conn = self.conn.clone();
        let raw: Option<String> = match conn.get(key).await {
            Ok(v) => v,
            Err(e) => {
                warn!(error = %e, key, "Redis GET failed");
                return None;
            }
        };

        let s = raw?;
        match serde_json::from_str(&s) {
            Ok(v) => Some(v),
            Err(e) => {
                warn!(error = %e, key, "Failed to deserialise cached value");
                None
            }
        }
    }

    /// Serialise and store a value with a TTL in seconds.
    ///
    /// Returns `true` on success. Failures are logged and the caller continues
    /// without the cached value.
    pub async fn set<T: Serialize>(&self, key: &str, value: &T, ttl_secs: u64) -> bool {
        let json = match serde_json::to_string(value) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, key, "Failed to serialise value for cache");
                return false;
            }
        };

        let mut conn = self.conn.clone();
        match conn
            .set_ex::<_, _, ()>(key, json, ttl_secs)
            .await
        {
            Ok(()) => true,
            Err(e) => {
                warn!(error = %e, key, "Redis SET failed");
                false
            }
        }
    }

    /// Send a `PING` to Redis and return `true` if it responds.
    ///
    /// Used by the health endpoint to surface Redis reachability without
    /// exposing the inner `ConnectionManager`.
    pub async fn ping(&self) -> bool {
        let mut conn = self.conn.clone();
        let result: Result<String, _> = redis::cmd("PING").query_async(&mut conn).await;
        result.is_ok()
    }
}

/// Build the cache key for a single event lookup.
pub fn event_key(source_id: &str) -> String {
    format!("api:event:{}", source_id)
}

/// Cache key for the stats response.
pub const STATS_KEY: &str = "api:stats";
