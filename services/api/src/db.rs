use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::RequestError;
use crate::model::{BandStats, EventResponse, EventsQuery};

/// Flat row returned by the events list query.
///
/// `total` is a window-function count of all matching rows (before pagination).
/// `latitude` and `longitude` are extracted from the PostGIS geometry column
/// via `ST_Y(location)::float8` and `ST_X(location)::float8` respectively.
#[derive(sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    source_id: String,
    source_network: String,
    event_time: DateTime<Utc>,
    latitude: f64,
    longitude: f64,
    depth_km: Option<f64>,
    magnitude: f64,
    magnitude_type: String,
    region_name: Option<String>,
    quality_indicator: String,
    processed_at: DateTime<Utc>,
    pipeline_version: String,
    /// COUNT(*) OVER() — total matching rows before LIMIT/OFFSET.
    total: i64,
}

impl From<EventRow> for EventResponse {
    fn from(r: EventRow) -> Self {
        EventResponse {
            id: r.id,
            source_id: r.source_id,
            source_network: r.source_network,
            event_time: r.event_time,
            latitude: r.latitude,
            longitude: r.longitude,
            depth_km: r.depth_km,
            magnitude: r.magnitude,
            magnitude_type: r.magnitude_type,
            region_name: r.region_name,
            quality_indicator: r.quality_indicator,
            processed_at: r.processed_at,
            pipeline_version: r.pipeline_version,
        }
    }
}

/// Build the SQL for a paginated, filtered events query.
///
/// Extracted so that the SQL can be inspected in tests without a live database.
/// Returns the `QueryBuilder` with all bind parameters attached; callers execute
/// it with `.build_query_as().fetch_all(pool)`.
///
/// `page` and `page_size` must already be clamped by the caller (the route
/// handler is the single place that enforces those bounds).
pub(crate) fn build_list_events_query(
    query: &EventsQuery,
    page: u32,
    page_size: u32,
) -> sqlx::QueryBuilder<'_, sqlx::Postgres> {
    let offset = (page - 1) as i64 * page_size as i64;

    // Build the query dynamically so that absent filters don't appear as
    // IS NOT NULL constraints and can use the table's indexes cleanly.
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        r#"SELECT
            id,
            source_id,
            source_network,
            event_time,
            ST_Y(location)::float8 AS latitude,
            ST_X(location)::float8 AS longitude,
            depth_km::float8 AS depth_km,
            magnitude::float8 AS magnitude,
            magnitude_type,
            region_name,
            quality_indicator,
            processed_at,
            pipeline_version,
            COUNT(*) OVER() AS total
        FROM seismology.seismic_events
        WHERE TRUE"#,
    );

    if let Some(start) = query.start_time {
        qb.push(" AND event_time >= ").push_bind(start);
    }
    if let Some(end) = query.end_time {
        qb.push(" AND event_time <= ").push_bind(end);
    }
    if let Some(min_mag) = query.min_magnitude {
        qb.push(" AND magnitude >= ").push_bind(min_mag);
    }
    if let Some(max_mag) = query.max_magnitude {
        qb.push(" AND magnitude <= ").push_bind(max_mag);
    }
    // Bounding box: all four corners must be provided together (validated by the
    // route handler before this function is called).
    if let (Some(min_lon), Some(min_lat), Some(max_lon), Some(max_lat)) =
        (query.min_lon, query.min_lat, query.max_lon, query.max_lat)
    {
        qb.push(" AND location && ST_MakeEnvelope(")
            .push_bind(min_lon)
            .push(", ")
            .push_bind(min_lat)
            .push(", ")
            .push_bind(max_lon)
            .push(", ")
            .push_bind(max_lat)
            .push(", 4326)");
    }

    qb.push(" ORDER BY event_time DESC LIMIT ")
        .push_bind(page_size as i64)
        .push(" OFFSET ")
        .push_bind(offset);

    qb
}

/// Fetch a paginated, filtered list of events.
///
/// Returns `(events, total_matching_rows)`. All filter parameters are optional;
/// when absent the corresponding WHERE clause is omitted entirely.
///
/// `page` and `page_size` must already be clamped by the caller.
///
/// Bounding-box filtering uses PostGIS's `&&` operator (bbox overlap) against
/// `ST_MakeEnvelope(min_lon, min_lat, max_lon, max_lat, 4326)`, which hits the
/// spatial index on `location`.
pub async fn list_events(
    pool: &PgPool,
    query: &EventsQuery,
    page: u32,
    page_size: u32,
) -> Result<(Vec<EventResponse>, i64), RequestError> {
    let mut qb = build_list_events_query(query, page, page_size);

    let rows: Vec<EventRow> = qb.build_query_as().fetch_all(pool).await?;

    let total = rows.first().map(|r| r.total).unwrap_or(0);
    let events = rows.into_iter().map(EventResponse::from).collect();

    Ok((events, total))
}

/// Fetch a single event by its external `source_id`.
///
/// Returns `None` when no matching row exists.
pub async fn get_event_by_source_id(
    pool: &PgPool,
    source_id: &str,
) -> Result<Option<EventResponse>, RequestError> {
    let row = sqlx::query_as!(
        EventRow,
        r#"
        SELECT
            id                                      AS "id!: Uuid",
            source_id                               AS "source_id!: String",
            source_network                          AS "source_network!: String",
            event_time                              AS "event_time!: DateTime<Utc>",
            ST_Y(location)::float8                  AS "latitude!: f64",
            ST_X(location)::float8                  AS "longitude!: f64",
            depth_km::float8                        AS "depth_km?: f64",
            magnitude::float8                       AS "magnitude!: f64",
            magnitude_type                          AS "magnitude_type!: String",
            region_name                             AS "region_name?: String",
            quality_indicator                       AS "quality_indicator!: String",
            processed_at                            AS "processed_at!: DateTime<Utc>",
            pipeline_version                        AS "pipeline_version!: String",
            1::bigint                               AS "total!: i64"
        FROM seismology.seismic_events
        WHERE source_id = $1
        "#,
        source_id,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(EventResponse::from))
}

/// Magnitude band definition used in the stats query.
pub(crate) struct Band {
    pub(crate) name: &'static str,
    pub(crate) min: f64,
    /// `None` for the open-ended top band (≥ 8.0).
    pub(crate) max: Option<f64>,
}

pub(crate) const BANDS: &[Band] = &[
    Band {
        name: "minor",
        min: 0.0,
        max: Some(2.0),
    },
    Band {
        name: "light",
        min: 2.0,
        max: Some(4.0),
    },
    Band {
        name: "moderate",
        min: 4.0,
        max: Some(6.0),
    },
    Band {
        name: "strong",
        min: 6.0,
        max: Some(8.0),
    },
    Band {
        name: "major",
        min: 8.0,
        max: None,
    },
];

/// Per-band, per-window stats row from the DB.
#[derive(sqlx::FromRow)]
struct StatsBandRow {
    count_1h: i64,
    count_24h: i64,
    count_7d: i64,
    count_30d: i64,
    max_mag_1h: Option<f64>,
    max_mag_24h: Option<f64>,
    max_mag_7d: Option<f64>,
    max_mag_30d: Option<f64>,
}

/// Query event counts and maximum magnitudes for each magnitude band and
/// each pre-defined time window (1h, 24h, 7d, 30d) relative to `now()`.
///
/// The "major" band (≥ 8.0) has no upper bound (`max = None`). The SQL uses
/// `($2::float8 IS NULL OR magnitude::float8 < $2::float8)` so that binding
/// `None` for `$2` correctly includes all magnitudes above the lower bound,
/// without relying on `f64::MAX` or `'Infinity'` sentinel values.
///
/// A CTE pre-filters to the band so the FILTER clauses in the aggregates
/// are concise and symmetric across all bands.
///
/// The query is not cached at this layer — callers are responsible for
/// Redis caching.
pub async fn get_stats(pool: &PgPool) -> Result<Vec<BandStats>, RequestError> {
    // language=sql
    const BAND_SQL: &str = r#"
        WITH band_events AS (
            SELECT magnitude::float8 AS mag, event_time
            FROM seismology.seismic_events
            WHERE magnitude::float8 >= $1
              AND ($2::float8 IS NULL OR magnitude::float8 < $2::float8)
        )
        SELECT
            COUNT(*) FILTER (WHERE event_time >= now() - INTERVAL '1 hour')
                AS count_1h,
            COUNT(*) FILTER (WHERE event_time >= now() - INTERVAL '24 hours')
                AS count_24h,
            COUNT(*) FILTER (WHERE event_time >= now() - INTERVAL '7 days')
                AS count_7d,
            COUNT(*) FILTER (WHERE event_time >= now() - INTERVAL '30 days')
                AS count_30d,
            MAX(mag) FILTER (WHERE event_time >= now() - INTERVAL '1 hour')
                AS max_mag_1h,
            MAX(mag) FILTER (WHERE event_time >= now() - INTERVAL '24 hours')
                AS max_mag_24h,
            MAX(mag) FILTER (WHERE event_time >= now() - INTERVAL '7 days')
                AS max_mag_7d,
            MAX(mag) FILTER (WHERE event_time >= now() - INTERVAL '30 days')
                AS max_mag_30d
        FROM band_events
    "#;

    let mut result = Vec::with_capacity(BANDS.len());

    for band in BANDS {
        // `band.max` is `None` for the open-ended top band. sqlx binds `None`
        // as SQL NULL, and `$2::float8 IS NULL` becomes true, making the upper
        // bound condition a no-op for that band.
        let row = sqlx::query_as::<_, StatsBandRow>(BAND_SQL)
            .bind(band.min)
            .bind(band.max) // Option<f64>: None → NULL
            .fetch_one(pool)
            .await?;

        result.push(BandStats {
            band: band.name.to_owned(),
            min_magnitude: band.min,
            max_magnitude: band.max,
            count_1h: row.count_1h,
            count_24h: row.count_24h,
            count_7d: row.count_7d,
            count_30d: row.count_30d,
            max_mag_1h: row.max_mag_1h,
            max_mag_24h: row.max_mag_24h,
            max_mag_7d: row.max_mag_7d,
            max_mag_30d: row.max_mag_30d,
        });
    }

    Ok(result)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventsQuery;

    // ── Band boundary tests ───────────────────────────────────────────────────

    /// Every adjacent pair of bands must share a boundary (max[n] == min[n+1])
    /// so there are no gaps or overlaps in the magnitude space.
    #[test]
    fn bands_are_contiguous() {
        for window in BANDS.windows(2) {
            let (a, b) = (&window[0], &window[1]);
            let a_max = a.max.expect("only the last band has max = None");
            assert_eq!(
                a_max, b.min,
                "band '{}' max ({}) != band '{}' min ({})",
                a.name, a_max, b.name, b.min
            );
        }
    }

    /// The first band must start at 0.0 (covers micro-earthquakes and noise).
    #[test]
    fn bands_start_at_zero() {
        assert_eq!(BANDS[0].min, 0.0, "first band should start at 0.0");
    }

    /// Only the last band should be open-ended (max = None).
    #[test]
    fn only_last_band_is_open_ended() {
        let last_idx = BANDS.len() - 1;
        for (i, band) in BANDS.iter().enumerate() {
            if i < last_idx {
                assert!(
                    band.max.is_some(),
                    "band '{}' at index {} should have an upper bound",
                    band.name,
                    i
                );
            } else {
                assert!(
                    band.max.is_none(),
                    "last band '{}' should be open-ended (max = None)",
                    band.name
                );
            }
        }
    }

    /// The "major" band threshold must be 8.0 per the documented API contract.
    #[test]
    fn major_band_threshold_is_eight() {
        let major = BANDS
            .iter()
            .find(|b| b.name == "major")
            .expect("major band must exist");
        assert_eq!(major.min, 8.0);
        assert!(major.max.is_none());
    }

    // ── QueryBuilder SQL tests ────────────────────────────────────────────────

    fn empty_query() -> EventsQuery {
        EventsQuery {
            page: None,
            page_size: None,
            start_time: None,
            end_time: None,
            min_magnitude: None,
            max_magnitude: None,
            min_lat: None,
            max_lat: None,
            min_lon: None,
            max_lon: None,
        }
    }

    #[test]
    fn no_filters_produces_no_extra_and_clauses() {
        let query = empty_query();
        let qb = build_list_events_query(&query, 1, 50);
        let sql = qb.sql();
        // Only the structural WHERE TRUE should be present; no dynamic AND clauses.
        assert!(sql.contains("WHERE TRUE"));
        assert!(!sql.contains("AND event_time"));
        assert!(!sql.contains("AND magnitude"));
        assert!(!sql.contains("AND location"));
    }

    #[test]
    fn start_time_filter_adds_clause() {
        let q = EventsQuery {
            start_time: Some(chrono::Utc::now()),
            ..empty_query()
        };
        let sql = build_list_events_query(&q, 1, 50).sql().to_owned();
        assert!(
            sql.contains("AND event_time >= "),
            "expected start_time clause, got: {sql}"
        );
    }

    #[test]
    fn end_time_filter_adds_clause() {
        let q = EventsQuery {
            end_time: Some(chrono::Utc::now()),
            ..empty_query()
        };
        let sql = build_list_events_query(&q, 1, 50).sql().to_owned();
        assert!(
            sql.contains("AND event_time <= "),
            "expected end_time clause, got: {sql}"
        );
    }

    #[test]
    fn magnitude_filters_add_clauses() {
        let q = EventsQuery {
            min_magnitude: Some(3.0),
            max_magnitude: Some(7.0),
            ..empty_query()
        };
        let sql = build_list_events_query(&q, 1, 50).sql().to_owned();
        assert!(
            sql.contains("AND magnitude >= "),
            "expected min_magnitude clause"
        );
        assert!(
            sql.contains("AND magnitude <= "),
            "expected max_magnitude clause"
        );
    }

    #[test]
    fn bbox_filter_adds_st_make_envelope() {
        let q = EventsQuery {
            min_lon: Some(-10.0),
            min_lat: Some(35.0),
            max_lon: Some(45.0),
            max_lat: Some(72.0),
            ..empty_query()
        };
        let sql = build_list_events_query(&q, 1, 50).sql().to_owned();
        assert!(
            sql.contains("ST_MakeEnvelope"),
            "expected ST_MakeEnvelope in bbox query, got: {sql}"
        );
    }

    #[test]
    fn partial_bbox_produces_no_spatial_clause() {
        // When only some bbox params are present the handler already rejects the
        // request. But even if called directly, the QueryBuilder should produce
        // no spatial clause because the `if let (Some, Some, Some, Some)` guard
        // won't match.
        let q = EventsQuery {
            min_lat: Some(35.0),
            ..empty_query() // max_lat, min_lon, max_lon all None
        };
        let sql = build_list_events_query(&q, 1, 50).sql().to_owned();
        assert!(
            !sql.contains("ST_MakeEnvelope"),
            "partial bbox must not inject ST_MakeEnvelope, got: {sql}"
        );
    }

    #[test]
    fn pagination_uses_supplied_page_and_page_size() {
        let sql = build_list_events_query(&empty_query(), 3, 25)
            .sql()
            .to_owned();
        // LIMIT and OFFSET placeholders appear in order as $N binds.
        assert!(sql.contains("LIMIT "), "expected LIMIT clause");
        assert!(sql.contains("OFFSET "), "expected OFFSET clause");
    }
}
