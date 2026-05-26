use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::RequestError;
use crate::model::{BandStats, EventResponse, EventsQuery};

/// Flat row returned by the events list query.
///
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
    /// Carries 1::bigint for the single-row `get_event_by_source_id` path.
    total: i64,
}

impl From<EventRow> for EventResponse {
    fn from(r: EventRow) -> Self {
        // `total` is selected as `1::bigint` in the single-row query to satisfy
        // the shared struct layout required by sqlx's FromRow derive.
        let _ = r.total;
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

/// Flat row for the cursor-based list query (no `total` window function).
#[derive(sqlx::FromRow)]
struct CursorRow {
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
}

impl From<CursorRow> for EventResponse {
    fn from(r: CursorRow) -> Self {
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

/// Decoded pagination cursor.
pub(crate) struct ParsedCursor {
    pub(crate) event_time: DateTime<Utc>,
    pub(crate) source_id: String,
}

/// JSON payload stored inside the base64url cursor token.
#[derive(Serialize, Deserialize)]
struct CursorPayload {
    et: DateTime<Utc>,
    sid: String,
}

/// Decode a base64url cursor string into a `ParsedCursor`.
/// Returns `None` for any malformed input.
pub(crate) fn decode_cursor(s: &str) -> Option<ParsedCursor> {
    let bytes = URL_SAFE_NO_PAD.decode(s).ok()?;
    let payload: CursorPayload = serde_json::from_slice(&bytes).ok()?;
    Some(ParsedCursor {
        event_time: payload.et,
        source_id: payload.sid,
    })
}

/// Encode a cursor from the last row's `event_time` and `source_id`.
fn encode_cursor(event_time: DateTime<Utc>, source_id: &str) -> String {
    let payload = CursorPayload {
        et: event_time,
        sid: source_id.to_owned(),
    };
    let json = serde_json::to_vec(&payload).expect("CursorPayload always serialises");
    URL_SAFE_NO_PAD.encode(json)
}

/// Build the SQL for a paginated, filtered events query using keyset cursor
/// pagination.
///
/// Fetches `page_size` rows ordered by `(event_time DESC, source_id DESC)`.
/// When `cursor` is provided, adds a WHERE clause that excludes all rows at or
/// before the cursor position, ensuring stable forward pagination without
/// duplicate or skipped rows as new events arrive.
///
/// The caller is responsible for fetching `page_size + 1` rows (to detect
/// `has_more`) by passing that inflated count as `page_size`.
///
/// Extracted so that the SQL can be inspected in tests without a live database.
pub(crate) fn build_list_events_query<'a>(
    query: &'a EventsQuery,
    cursor: Option<&'a ParsedCursor>,
    page_size: u32,
) -> sqlx::QueryBuilder<'a, sqlx::Postgres> {
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
            pipeline_version
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

    // Keyset cursor: skip all rows that are at or before the cursor position.
    // The compound condition (event_time < $et) OR (event_time = $et AND source_id < $sid)
    // correctly handles ties in event_time without skipping or duplicating rows.
    if let Some(c) = cursor {
        qb.push(" AND (event_time < ")
            .push_bind(c.event_time)
            .push(" OR (event_time = ")
            .push_bind(c.event_time)
            .push(" AND source_id < ")
            .push_bind(c.source_id.clone())
            .push("))");
    }

    qb.push(" ORDER BY event_time DESC, source_id DESC LIMIT ")
        .push_bind(page_size as i64);

    qb
}

/// Fetch a paginated, filtered list of events using keyset cursor pagination.
///
/// Returns `(events, has_more, next_cursor)`. Fetch one extra row internally
/// to detect whether more results exist; `next_cursor` is only present when
/// `has_more` is true.
pub async fn list_events(
    pool: &PgPool,
    query: &EventsQuery,
    cursor: Option<&ParsedCursor>,
    page_size: u32,
) -> Result<(Vec<EventResponse>, bool, Option<String>), RequestError> {
    // Fetch one extra row to detect has_more without a COUNT query.
    let fetch_size = page_size.saturating_add(1);
    let mut qb = build_list_events_query(query, cursor, fetch_size);

    let mut rows: Vec<CursorRow> = qb.build_query_as().fetch_all(pool).await?;

    let has_more = rows.len() > page_size as usize;
    if has_more {
        rows.truncate(page_size as usize);
    }

    let next_cursor = if has_more {
        rows.last()
            .map(|r| encode_cursor(r.event_time, &r.source_id))
    } else {
        None
    };

    let events = rows.into_iter().map(EventResponse::from).collect();
    Ok((events, has_more, next_cursor))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EventsQuery;

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

    fn empty_query() -> EventsQuery {
        EventsQuery {
            cursor: None,
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
        let qb = build_list_events_query(&query, None, 50);
        let sql = qb.sql();
        assert!(sql.contains("WHERE TRUE"));
        assert!(!sql.contains("AND event_time"));
        assert!(!sql.contains("AND magnitude"));
        assert!(!sql.contains("AND location"));
    }

    #[test]
    fn no_cursor_produces_no_keyset_clause() {
        let sql = build_list_events_query(&empty_query(), None, 25)
            .sql()
            .to_owned();
        assert!(sql.contains("LIMIT "), "expected LIMIT clause");
        assert!(
            !sql.contains("OFFSET "),
            "cursor pagination must not use OFFSET"
        );
        assert!(
            !sql.contains("source_id <"),
            "no cursor means no keyset clause"
        );
    }

    #[test]
    fn cursor_adds_keyset_clause() {
        let cursor = ParsedCursor {
            event_time: chrono::Utc::now(),
            source_id: "test-id".to_owned(),
        };
        let sql = build_list_events_query(&empty_query(), Some(&cursor), 25)
            .sql()
            .to_owned();
        assert!(
            sql.contains("event_time <"),
            "expected keyset time clause, got: {sql}"
        );
        assert!(
            sql.contains("source_id <"),
            "expected keyset id clause, got: {sql}"
        );
    }

    #[test]
    fn cursor_roundtrip() {
        let et = chrono::Utc::now();
        let sid = "us6000abcd";
        let encoded = encode_cursor(et, sid);
        let decoded = decode_cursor(&encoded).expect("roundtrip must succeed");
        // chrono serialises to microsecond precision; compare within 1 µs.
        assert_eq!(decoded.event_time.timestamp_micros(), et.timestamp_micros());
        assert_eq!(decoded.source_id, sid);
    }

    #[test]
    fn invalid_cursor_returns_none() {
        assert!(decode_cursor("!!!not-base64!!!").is_none());
        assert!(decode_cursor("aGVsbG8=").is_none()); // valid base64 but not JSON cursor
    }

    #[test]
    fn start_time_filter_adds_clause() {
        let q = EventsQuery {
            start_time: Some(chrono::Utc::now()),
            ..empty_query()
        };
        let sql = build_list_events_query(&q, None, 50).sql().to_owned();
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
        let sql = build_list_events_query(&q, None, 50).sql().to_owned();
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
        let sql = build_list_events_query(&q, None, 50).sql().to_owned();
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
        let sql = build_list_events_query(&q, None, 50).sql().to_owned();
        assert!(
            sql.contains("ST_MakeEnvelope"),
            "expected ST_MakeEnvelope in bbox query, got: {sql}"
        );
    }

    #[test]
    fn partial_bbox_produces_no_spatial_clause() {
        let q = EventsQuery {
            min_lat: Some(35.0),
            ..empty_query()
        };
        let sql = build_list_events_query(&q, None, 50).sql().to_owned();
        assert!(
            !sql.contains("ST_MakeEnvelope"),
            "partial bbox must not inject ST_MakeEnvelope, got: {sql}"
        );
    }
}
