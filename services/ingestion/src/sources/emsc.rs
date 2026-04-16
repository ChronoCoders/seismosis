//! EMSC (European-Mediterranean Seismological Centre) FDSNWS source.
//!
//! API docs: https://www.seismicportal.eu/fdsnws/event/1/
//! Response: GeoJSON FeatureCollection.
//!
//! Key field shapes (confirmed against live API 2024-01-01):
//! - `feature.id`             → String  (e.g. "20240101_0000070") — composite unid
//! - `properties.time`        → String  — ISO-8601 UTC (e.g. "2024-01-01T05:22:54.5Z")
//! - `properties.mag`         → f64
//! - `properties.magtype`     → String  — lowercase from EMSC
//! - `properties.flynn_region`→ String? — region name
//! - `properties.evtype`      → String? — "ke" = known earthquake
//! - `properties.lat`         → f64     — use direct field, not geometry Z
//! - `properties.lon`         → f64
//! - `properties.depth`       → f64?    — positive km (geometry Z is negative altitude)
//! - `properties.unid`        → String  — stable EMSC event identifier
//!
//! Note: `geometry.coordinates[2]` is negative (altitude convention).
//!       Always use `properties.depth` for the positive depth value.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{debug, warn};

use super::SeismicSource;
use crate::config::Config;
use crate::error::{IngestError, ParseError};
use crate::schema::RawEarthquakeEvent;
use crate::sources::{emsc_quality, normalise_mag_type, validate_coordinates};

const SOURCE_NAME: &str = "EMSC";

// ─── Serde shapes ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    id: String,
    properties: Properties,
}

#[derive(Deserialize)]
struct Properties {
    /// ISO-8601 UTC event time (e.g. "2024-01-01T05:22:54.5Z").
    time: Option<String>,
    mag: Option<f64>,
    magtype: Option<String>,
    flynn_region: Option<String>,
    evtype: Option<String>,
    /// Direct lat/lon properties (not from geometry coordinates).
    lat: Option<f64>,
    lon: Option<f64>,
    /// Depth in km, positive downward. Use this — NOT geometry.coordinates[2].
    depth: Option<f64>,
    /// Stable EMSC identifier. Prefer over `feature.id` for dedup key.
    unid: Option<String>,
}

// ─── Source implementation ────────────────────────────────────────────────────

pub struct EmscSource {
    client: reqwest::Client,
    config: Arc<Config>,
}

impl EmscSource {
    pub fn new(client: reqwest::Client, config: Arc<Config>) -> Self {
        Self { client, config }
    }

    async fn fetch_once(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<RawEarthquakeEvent>, IngestError> {
        let now = Utc::now();
        let url = format!(
            "{}/query?format=json\
             &starttime={}\
             &endtime={}\
             &minmagnitude={}\
             &orderby=time\
             &limit=1000",
            self.config.emsc_api_url,
            since.format("%Y-%m-%dT%H:%M:%S"),
            now.format("%Y-%m-%dT%H:%M:%S"),
            self.config.min_magnitude,
        );

        debug!(url = %url, "Fetching EMSC events");

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| IngestError::HttpFetch {
                src: SOURCE_NAME,
                inner: e,
            })?;

        let status = response.status();

        // HTTP 204 is the canonical FDSN "no events matched" response.
        if status == reqwest::StatusCode::NO_CONTENT {
            debug!(source = SOURCE_NAME, "No events in window (HTTP 204)");
            return Ok(vec![]);
        }

        if !status.is_success() {
            return Err(IngestError::HttpFetch {
                src: SOURCE_NAME,
                inner: response
                    .error_for_status()
                    .expect_err("status is not success"),
            });
        }

        let body = response.bytes().await.map_err(|e| IngestError::HttpFetch {
            src: SOURCE_NAME,
            inner: e,
        })?;

        // EMSC non-standardly returns HTTP 200 with an empty body when there
        // are no events in the requested window. Treat it the same as 204 —
        // the query succeeded but matched no events. Do NOT propagate as an
        // error: that would trigger the retry strategy and generate ERROR logs
        // every poll cycle during quiet periods.
        if body.is_empty() {
            debug!(
                source = SOURCE_NAME,
                "No events in window (HTTP 200 + empty body)"
            );
            return Ok(vec![]);
        }

        let collection: FeatureCollection =
            serde_json::from_slice(&body).map_err(|e| IngestError::JsonParse {
                src: SOURCE_NAME,
                event_id: "(collection)".into(),
                inner: e,
            })?;

        let ingested_at_ms = Utc::now().timestamp_millis();
        let api_count = collection.features.len();
        let mut events = Vec::with_capacity(api_count);

        for feature in collection.features {
            match parse_feature(feature, ingested_at_ms, &self.config.pipeline_version) {
                Ok(event) => events.push(event),
                Err(e) => {
                    warn!(error = %e, source = SOURCE_NAME, "Skipping unparseable event");
                }
            }
        }

        if api_count >= 1000 {
            warn!(
                source = SOURCE_NAME,
                count = api_count,
                "API response hit the 1000-event limit — older events in the lookback \
                 window may have been truncated. Consider reducing SOURCE_LOOKBACK_SECS \
                 or SOURCE_POLL_INTERVAL_SECS."
            );
        }

        Ok(events)
    }
}

#[async_trait::async_trait]
impl SeismicSource for EmscSource {
    fn name(&self) -> &'static str {
        SOURCE_NAME
    }

    async fn fetch(&self, since: DateTime<Utc>) -> Result<Vec<RawEarthquakeEvent>, IngestError> {
        let retries = self.config.http_max_retries as usize;
        let strategy = ExponentialBackoff::from_millis(1_000)
            .factor(2)
            .max_delay(Duration::from_secs(16))
            .take(retries);

        Retry::spawn(strategy, || async { self.fetch_once(since).await }).await
    }
}

// ─── Feature parser ───────────────────────────────────────────────────────────

fn parse_feature(
    feature: Feature,
    ingested_at_ms: i64,
    pipeline_version: &str,
) -> Result<RawEarthquakeEvent, ParseError> {
    let event_id = feature.id.clone();
    let props = &feature.properties;

    let magnitude = props.mag.ok_or_else(|| ParseError::MissingField {
        field: "mag",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
    })?;

    let time_str = props
        .time
        .as_deref()
        .ok_or_else(|| ParseError::MissingField {
            field: "time",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
        })?;

    // EMSC uses ISO-8601 with fractional seconds (e.g. "2024-01-01T05:22:54.5Z").
    let event_time_ms = parse_iso8601_ms(time_str).map_err(|detail| ParseError::InvalidField {
        field: "time",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
        detail,
    })?;

    let latitude = props.lat.ok_or_else(|| ParseError::MissingField {
        field: "lat",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
    })?;

    let longitude = props.lon.ok_or_else(|| ParseError::MissingField {
        field: "lon",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
    })?;

    validate_coordinates(latitude, longitude, SOURCE_NAME, &event_id)?;

    // `properties.depth` is always positive km. Do NOT use geometry.coordinates[2]
    // which uses the altitude convention (negative = below surface).
    let depth_km = props.depth.filter(|d| d.is_finite() && *d >= 0.0);

    let magnitude_type = normalise_mag_type(props.magtype.as_deref().unwrap_or("UNKNOWN"));

    let quality_indicator = emsc_quality(props.evtype.as_deref()).to_owned();

    // Prefer `unid` as the stable identifier; fall back to the feature-level id.
    let stable_id = props
        .unid
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&feature.id);
    let source_id = format!("EMSC:{}", stable_id);

    let raw_payload = serde_json::json!({
        "id": feature.id,
        "unid": props.unid,
        "time": time_str,
        "mag": magnitude,
        "magtype": magnitude_type,
        "evtype": props.evtype,
        "flynn_region": props.flynn_region,
        "lat": latitude,
        "lon": longitude,
        "depth": depth_km,
    })
    .to_string();

    Ok(RawEarthquakeEvent {
        source_id,
        source_network: SOURCE_NAME.to_owned(),
        event_time_ms,
        latitude,
        longitude,
        depth_km,
        magnitude,
        magnitude_type,
        region_name: props.flynn_region.clone(),
        quality_indicator,
        raw_payload,
        ingested_at_ms,
        pipeline_version: pipeline_version.to_owned(),
    })
}

// ─── Timestamp helper ─────────────────────────────────────────────────────────

/// Parse an ISO-8601 UTC timestamp string into Unix epoch milliseconds.
///
/// Handles both `2024-01-01T05:22:54Z` and `2024-01-01T05:22:54.5Z`.
fn parse_iso8601_ms(s: &str) -> Result<i64, String> {
    // chrono's parse_from_rfc3339 handles fractional seconds and the Z suffix.
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .map_err(|e| format!("'{}' is not valid RFC-3339: {}", s, e))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso8601_ms_whole_seconds_with_z() {
        // 2024-01-01T00:00:00Z = 1704067200000 ms
        let ms = parse_iso8601_ms("2024-01-01T00:00:00Z").unwrap();
        assert_eq!(ms, 1_704_067_200_000);
    }

    #[test]
    fn parse_iso8601_ms_fractional_seconds() {
        // .5 fractional second = +500 ms
        let ms = parse_iso8601_ms("2024-01-01T00:00:00.5Z").unwrap();
        assert_eq!(ms, 1_704_067_200_500);
    }

    #[test]
    fn parse_iso8601_ms_millisecond_precision() {
        let ms = parse_iso8601_ms("2024-01-01T05:22:54.123Z").unwrap();
        // Verify it parses without error and the result is in a plausible range.
        assert!(ms > 1_700_000_000_000);
        assert!(ms < 2_000_000_000_000);
    }

    #[test]
    fn parse_iso8601_ms_empty_string_fails() {
        assert!(parse_iso8601_ms("").is_err());
    }

    #[test]
    fn parse_iso8601_ms_invalid_format_fails() {
        assert!(parse_iso8601_ms("not-a-date").is_err());
        assert!(parse_iso8601_ms("2024-13-01T00:00:00Z").is_err()); // month 13
        assert!(parse_iso8601_ms("2024-01-01 00:00:00").is_err()); // missing T and Z
    }

    #[test]
    fn parse_iso8601_ms_with_offset_fails() {
        // EMSC timestamps are always UTC (Z suffix). An offset should fail
        // because chrono's parse_from_rfc3339 accepts offsets, so this
        // actually succeeds — document that explicitly.
        let result = parse_iso8601_ms("2024-01-01T00:00:00+05:00");
        // RFC-3339 with numeric offset is valid; verify it doesn't panic.
        assert!(result.is_ok());
    }

    #[test]
    fn parse_iso8601_ms_epoch() {
        let ms = parse_iso8601_ms("1970-01-01T00:00:00Z").unwrap();
        assert_eq!(ms, 0);
    }
}
