use std::time::Instant;

use redis::AsyncCommands as _;
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::debug;

use crate::{
    config::Config,
    error::{ProcessError, StorageError},
    metrics::Metrics,
    model::EarthquakeEvent,
};

/// Create the connection pool and verify connectivity.
pub async fn create_pool(config: &Config) -> Result<PgPool, StorageError> {
    PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(config.db_connect_timeout)
        .connect(&config.database_url)
        .await
        .map_err(StorageError::Database)
}

/// A matching event found by the cross-source proximity query.
pub struct CrossSourceMatch {
    pub source_id: String,
    pub quality_indicator: String,
}

/// Query for an existing event from a *different* source network that falls
/// within the cross-source deduplication window:
/// - event_time within 30 seconds, AND
/// - coordinates within 0.5° on both axes.
///
/// Uses a runtime `sqlx::query_as` (not the `query!` macro) to avoid requiring
/// a live database during compilation.
pub async fn find_cross_source_match(
    pool: &PgPool,
    event: &EarthquakeEvent,
) -> Result<Option<CrossSourceMatch>, ProcessError> {
    const SQL: &str = r#"
        SELECT source_id, quality_indicator
        FROM seismology.seismic_events
        WHERE source_network != $1
          AND ABS(EXTRACT(EPOCH FROM (event_time - $2))) <= 30
          AND ST_DWithin(
                location::geography,
                ST_SetSRID(ST_MakePoint($3, $4), 4326)::geography,
                55000
              )
        LIMIT 1
    "#;

    #[derive(sqlx::FromRow)]
    struct Row {
        source_id: String,
        quality_indicator: String,
    }

    let row = sqlx::query_as::<_, Row>(SQL)
        .bind(&event.source_network)
        .bind(event.event_time)
        .bind(event.longitude)
        .bind(event.latitude)
        .fetch_optional(pool)
        .await
        .map_err(|e| ProcessError::DbQuery(e.to_string()))?;

    Ok(row.map(|r| CrossSourceMatch {
        source_id: r.source_id,
        quality_indicator: r.quality_indicator,
    }))
}

/// Map a quality indicator character to a numeric rank for comparison.
/// Higher rank = better quality. A(4) > B(3) > C(2) > D(1).
pub fn quality_rank(q: &str) -> u8 {
    match q {
        "A" => 4,
        "B" => 3,
        "C" => 2,
        "D" => 1,
        other => unreachable!("quality_rank called with unvalidated indicator: {other:?}"),
    }
}

/// Upsert a validated event into `seismology.seismic_events`.
///
/// Uses `ON CONFLICT (source_id) DO UPDATE SET ... WHERE EXCLUDED.event_time >
/// seismic_events.event_time` so the operation is idempotent and monotonic:
///
/// - **New `source_id`**: always inserts.
/// - **Existing `source_id`, incoming event is newer**: updates all columns.
/// - **Existing `source_id`, incoming event is same age or older**: silently
///   skips the update. PostgreSQL returns 0 rows affected; no error is raised.
///   This handles late-arriving duplicates from overlapping source poll windows.
///
/// After a successful upsert, fires a best-effort `DEL api:event:{source_id}`
/// against Redis to evict any stale cached response. The DEL is spawned as a
/// background task; a failure is logged at WARN and does not affect the result.
///
/// # Type coercions
/// - `magnitude`, `depth_km`: Avro `double` → `$n::float8` cast tells
///   PostgreSQL the parameter type, which then coerces implicitly to
///   `NUMERIC(4,2)` / `NUMERIC(8,3)`. Validation in `model.rs` guarantees
///   the values fit the column precision before reaching this call.
/// - `latitude` / `longitude`: passed as `float8` parameters and assembled
///   into the `GEOMETRY(POINT, 4326)` location column via
///   `ST_SetSRID(ST_MakePoint($lon, $lat), 4326)`.
/// - `raw_payload`: `serde_json::Value` → `JSONB` via sqlx's `json` feature.
/// - `event_time`, `processed_at`: `DateTime<Utc>` → `TIMESTAMPTZ`.
pub async fn upsert_event(
    pool: &PgPool,
    event: &EarthquakeEvent,
    metrics: &Metrics,
    redis: Option<redis::aio::ConnectionManager>,
) -> Result<(), ProcessError> {
    let db_start = Instant::now();

    sqlx::query!(
        r#"
        INSERT INTO seismology.seismic_events (
            source_id,
            source_network,
            event_time,
            magnitude,
            magnitude_type,
            location,
            depth_km,
            region_name,
            quality_indicator,
            raw_payload,
            processed_at,
            pipeline_version
        )
        VALUES (
            $1,
            $2,
            $3,
            $4::float8,
            $5,
            ST_SetSRID(ST_MakePoint($6::float8, $7::float8), 4326),
            $8::float8,
            $9,
            $10,
            $11,
            $12,
            $13
        )
        ON CONFLICT (source_id) DO UPDATE SET
            source_network    = EXCLUDED.source_network,
            event_time        = EXCLUDED.event_time,
            magnitude         = EXCLUDED.magnitude,
            magnitude_type    = EXCLUDED.magnitude_type,
            location          = EXCLUDED.location,
            depth_km          = EXCLUDED.depth_km,
            region_name       = EXCLUDED.region_name,
            quality_indicator = EXCLUDED.quality_indicator,
            raw_payload       = EXCLUDED.raw_payload,
            processed_at      = EXCLUDED.processed_at,
            pipeline_version  = EXCLUDED.pipeline_version
        WHERE EXCLUDED.event_time > seismic_events.event_time
        "#,
        event.source_id,                             // $1  TEXT
        event.source_network,                        // $2  TEXT
        event.event_time,                            // $3  TIMESTAMPTZ
        event.magnitude as f64,                      // $4  → ::float8 → NUMERIC(4,2)
        event.magnitude_type,                        // $5  TEXT
        event.longitude as f64,                      // $6  → X (longitude) in ST_MakePoint($6, $7)
        event.latitude as f64,                       // $7  → Y (latitude)  in ST_MakePoint($6, $7)
        event.depth_km as Option<f64>,               // $8  → ::float8 → NUMERIC(8,3) nullable
        event.region_name.clone() as Option<String>, // $9  TEXT nullable
        event.quality_indicator,                     // $10 CHAR(1)
        event.raw_payload,                           // $11 JSONB
        event.processed_at,                          // $12 TIMESTAMPTZ
        event.pipeline_version,                      // $13 TEXT
    )
    .execute(pool)
    .await
    .map_err(|e| ProcessError::DbUpsert(e.to_string()))?;

    let elapsed = db_start.elapsed().as_secs_f64();
    metrics
        .db_write_duration_seconds
        .with_label_values(&["seismic_events"])
        .observe(elapsed);

    metrics
        .events_upserted_total
        .with_label_values(&["upsert", magnitude_class(event.magnitude)])
        .inc();

    debug!(
        source_id = %event.source_id,
        magnitude = event.magnitude,
        elapsed_ms = (elapsed * 1000.0) as u64,
        "Event upserted"
    );

    // Evict the API cache entry so the next GET /v1/events/{id} fetches fresh
    // data. Fire-and-forget: a Redis failure should never fail a successful DB
    // write.
    if let Some(mut conn) = redis {
        let cache_key = format!("api:event:{}", event.source_id);
        // Fire-and-forget: the spawned task cannot panic — `del()` is the only
        // fallible operation and its Err path is handled with a warn!. The
        // JoinHandle is detached intentionally; we do not need to await it.
        drop(tokio::spawn(async move {
            if let Err(e) = conn.del::<_, ()>(&cache_key).await {
                tracing::warn!(key = %cache_key, error = %e, "Redis DEL failed");
            }
        }));
    }

    Ok(())
}

/// Map a magnitude value to a human-readable class label.
///
/// Breakpoints follow the USGS earthquake magnitude scale:
/// - minor:    M < 3.0
/// - light:    3.0 ≤ M < 4.0
/// - moderate: 4.0 ≤ M < 5.0
/// - strong:   5.0 ≤ M < 7.0
/// - major:    M ≥ 7.0
fn magnitude_class(magnitude: f64) -> &'static str {
    if magnitude < 3.0 {
        "minor"
    } else if magnitude < 4.0 {
        "light"
    } else if magnitude < 5.0 {
        "moderate"
    } else if magnitude < 7.0 {
        "strong"
    } else {
        "major"
    }
}
