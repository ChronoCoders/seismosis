use crate::error::ApiError;

/// Runtime configuration loaded from environment variables.
///
/// All variables have defaults suitable for local `docker compose` development.
/// Override them in the container environment or a `.env` file.
#[derive(Debug, Clone)]
pub struct Config {
    /// PostgreSQL connection string.
    pub database_url: String,
    /// Maximum simultaneous DB connections in the pool.
    pub db_max_connections: u32,
    /// Redis connection URL, e.g. `redis://redis:6379`.
    pub redis_url: String,
    /// TCP port the Axum HTTP server binds on.
    /// `/metrics` is served on this same port in Phase 1.
    pub http_port: u16,
    /// Maximum wall-clock time (seconds) allowed for a single HTTP request
    /// before the server returns `408 Request Timeout`.
    pub request_timeout_secs: u64,
    /// TTL (seconds) for per-event Redis cache entries.
    pub event_cache_ttl_secs: u64,
    /// TTL (seconds) for the stats Redis cache entry.
    pub stats_cache_ttl_secs: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, ApiError> {
        Ok(Config {
            database_url: var_str(
                "DATABASE_URL",
                "postgres://seismosis:seismosis@postgres:5432/seismosis",
            )?,
            db_max_connections: {
                let v = var_u64("DB_MAX_CONNECTIONS", 20)?;
                u32::try_from(v).map_err(|_| {
                    ApiError::Config(format!(
                        "DB_MAX_CONNECTIONS value {} overflows u32",
                        v
                    ))
                })?
            },
            redis_url: var_str("REDIS_URL", "redis://redis:6379")?,
            http_port: {
                let v = var_u64("HTTP_PORT", 8000)?;
                u16::try_from(v).map_err(|_| {
                    ApiError::Config(format!("HTTP_PORT value {} overflows u16", v))
                })?
            },
            request_timeout_secs: var_u64("REQUEST_TIMEOUT_SECS", 30)?,
            event_cache_ttl_secs: var_u64("EVENT_CACHE_TTL_SECS", 300)?,
            stats_cache_ttl_secs: var_u64("STATS_CACHE_TTL_SECS", 60)?,
        })
    }
}

fn var_str(name: &str, default: &str) -> Result<String, ApiError> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Ok(v),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(e) => Err(ApiError::Config(format!("{}: {}", name, e))),
    }
}

fn var_u64(name: &str, default: u64) -> Result<u64, ApiError> {
    match std::env::var(name) {
        Ok(v) if v.is_empty() => Ok(default),
        Ok(v) => v.parse::<u64>().map_err(|e| {
            ApiError::Config(format!("{}: expected integer, got {:?}: {}", name, v, e))
        }),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(e) => Err(ApiError::Config(format!("{}: {}", name, e))),
    }
}
