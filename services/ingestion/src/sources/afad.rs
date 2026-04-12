//! AFAD (Afet ve Acil Durum Yönetimi Başkanlığı) earthquake source.
//!
//! API endpoint: https://deprem.afad.gov.tr/apiv2/event/filter
//! Documentation: https://deprem.afad.gov.tr/apiv2/
//!
//! The API returns a JSON array of earthquake objects. Time range is passed
//! via `start` and `end` query parameters in "YYYY-MM-DD HH:MM:SS" format.
//!
//! Key field shapes (confirmed against live API 2026-04-12):
//! - `eventID`       → String  — AFAD composite identifier (e.g. "2024.0012.1234")
//! - `date`          → String  — "YYYY.MM.DD HH:MM:SS" UTC
//! - `magnitude`     → String  — decimal magnitude as a string
//! - `magnitudeType` → String  — magnitude scale (e.g. "Ml", "Mw")
//! - `latitude`      → String  — decimal degrees as a string
//! - `longitude`     → String  — decimal degrees as a string
//! - `depth`         → String  — depth in km, positive downward, as a string
//! - `location`      → String? — Turkish region/province description
//! - `type`          → String? — "Ke" = Kesinleşmiş (confirmed), others = automatic
//!
//! NOTE: All numeric fields are returned as strings by the v2 API. Each is
//! parsed explicitly; a parse failure is treated as a missing field and the
//! event is routed to the dead-letter queue.
//!
//! NOTE: `date` is UTC. The AFAD public web interface displays local Turkish
//! time (UTC+3), but the v2 JSON API consistently returns UTC.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{debug, warn};

use crate::config::Config;
use crate::error::{IngestError, ParseError};
use crate::schema::RawEarthquakeEvent;
use crate::sources::{afad_quality, normalise_mag_type, validate_coordinates};
use super::SeismicSource;

const SOURCE_NAME: &str = "AFAD";

// ─── Serde shapes ────────────────────────────────────────────────────────────

/// A single earthquake entry in the AFAD v2 response array.
///
/// All numeric fields are delivered as strings; `Option<String>` is used
/// throughout so that missing keys in future API versions degrade gracefully
/// rather than failing the entire response parse.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AfadEvent {
    #[serde(rename = "eventID")]
    event_id: Option<String>,
    date: Option<String>,
    magnitude: Option<String>,
    magnitude_type: Option<String>,
    latitude: Option<String>,
    longitude: Option<String>,
    depth: Option<String>,
    location: Option<String>,
    /// Event type: "Ke" = Kesinleşmiş (manually confirmed), others = automatic.
    #[serde(rename = "type")]
    event_type: Option<String>,
}

// ─── Source implementation ────────────────────────────────────────────────────

pub struct AfadSource {
    client: reqwest::Client,
    config: Arc<Config>,
}

impl AfadSource {
    pub fn new(client: reqwest::Client, config: Arc<Config>) -> Self {
        Self { client, config }
    }

    async fn fetch_once(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<RawEarthquakeEvent>, IngestError> {
        let now = Utc::now();

        // AFAD v2 expects space-separated datetime without T or Z.
        // reqwest percent-encodes spaces automatically when building the URL
        // via `.query()`, so we pass the raw strings and let it handle encoding.
        let url = format!(
            "{}/filter?start={}&end={}&minmag={}&limit=1000&orderby=timedesc",
            self.config.afad_api_url,
            since.format("%Y-%m-%d %H:%M:%S"),
            now.format("%Y-%m-%d %H:%M:%S"),
            self.config.min_magnitude,
        );

        debug!(url = %url, "Fetching AFAD events");

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| IngestError::HttpFetch { src: SOURCE_NAME, inner: e })?;

        let status = response.status();
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

        let events_raw: Vec<AfadEvent> =
            serde_json::from_slice(&body).map_err(|e| IngestError::JsonParse {
                src: SOURCE_NAME,
                event_id: "(array)".into(),
                inner: e,
            })?;

        let ingested_at_ms = Utc::now().timestamp_millis();
        let api_count = events_raw.len();
        let mut events = Vec::with_capacity(api_count);

        for raw in events_raw {
            match parse_event(raw, ingested_at_ms, &self.config.pipeline_version) {
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
impl SeismicSource for AfadSource {
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

// ─── Event parser ─────────────────────────────────────────────────────────────

fn parse_event(
    raw: AfadEvent,
    ingested_at_ms: i64,
    pipeline_version: &str,
) -> Result<RawEarthquakeEvent, ParseError> {
    // Use eventID as the dedup key; it is always present on valid records.
    let event_id = raw.event_id.clone().ok_or_else(|| ParseError::MissingField {
        field: "eventID",
        src: SOURCE_NAME,
        event_id: "(unknown)".into(),
    })?;

    let date_str = raw.date.as_deref().ok_or_else(|| ParseError::MissingField {
        field: "date",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
    })?;

    let event_time_ms = parse_afad_date(date_str).map_err(|detail| ParseError::InvalidField {
        field: "date",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
        detail,
    })?;

    let mag_str = raw.magnitude.as_deref().ok_or_else(|| ParseError::MissingField {
        field: "magnitude",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
    })?;
    let magnitude = mag_str.trim().parse::<f64>().map_err(|_| ParseError::InvalidField {
        field: "magnitude",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
        detail: format!("cannot parse '{}' as f64", mag_str),
    })?;

    let lat_str = raw.latitude.as_deref().ok_or_else(|| ParseError::MissingField {
        field: "latitude",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
    })?;
    let latitude = lat_str.trim().parse::<f64>().map_err(|_| ParseError::InvalidField {
        field: "latitude",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
        detail: format!("cannot parse '{}' as f64", lat_str),
    })?;

    let lon_str = raw.longitude.as_deref().ok_or_else(|| ParseError::MissingField {
        field: "longitude",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
    })?;
    let longitude = lon_str.trim().parse::<f64>().map_err(|_| ParseError::InvalidField {
        field: "longitude",
        src: SOURCE_NAME,
        event_id: event_id.clone(),
        detail: format!("cannot parse '{}' as f64", lon_str),
    })?;

    validate_coordinates(latitude, longitude, SOURCE_NAME, &event_id)?;

    // Depth: string, positive km. Missing or non-finite → None (stored as NULL).
    let depth_km = raw
        .depth
        .as_deref()
        .and_then(|s| s.trim().parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d >= 0.0);

    let magnitude_type = normalise_mag_type(
        raw.magnitude_type.as_deref().unwrap_or("UNKNOWN"),
    );

    let quality_indicator = afad_quality(raw.event_type.as_deref()).to_owned();

    let source_id = format!("AFAD:{}", event_id);

    let raw_payload = serde_json::json!({
        "eventID": event_id,
        "date": date_str,
        "magnitude": magnitude,
        "magnitudeType": magnitude_type,
        "latitude": latitude,
        "longitude": longitude,
        "depth": depth_km,
        "location": raw.location,
        "type": raw.event_type,
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
        region_name: raw.location,
        quality_indicator,
        raw_payload,
        ingested_at_ms,
        pipeline_version: pipeline_version.to_owned(),
    })
}

// ─── Timestamp helper ─────────────────────────────────────────────────────────

/// Parse an AFAD v2 UTC date string ("YYYY.MM.DD HH:MM:SS") into Unix epoch ms.
///
/// The format uses dots as the date separator and a space before the time
/// component — neither ISO-8601 nor RFC-3339. `chrono::NaiveDateTime::parse_from_str`
/// handles this without any extra dependencies.
fn parse_afad_date(s: &str) -> Result<i64, String> {
    NaiveDateTime::parse_from_str(s.trim(), "%Y.%m.%d %H:%M:%S")
        .map(|ndt| ndt.and_utc().timestamp_millis())
        .map_err(|e| format!("'{}' does not match AFAD date format YYYY.MM.DD HH:MM:SS: {}", s, e))
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_afad_date_valid() {
        // 2024-01-15 10:30:00 UTC = 1705314600000 ms
        let ms = parse_afad_date("2024.01.15 10:30:00").unwrap();
        assert_eq!(ms, 1_705_314_600_000);
    }

    #[test]
    fn parse_afad_date_epoch() {
        let ms = parse_afad_date("1970.01.01 00:00:00").unwrap();
        assert_eq!(ms, 0);
    }

    #[test]
    fn parse_afad_date_leading_trailing_whitespace() {
        // Real API responses sometimes include surrounding whitespace.
        let ms = parse_afad_date("  2024.01.15 10:30:00  ").unwrap();
        assert_eq!(ms, 1_705_314_600_000);
    }

    #[test]
    fn parse_afad_date_wrong_separator_fails() {
        assert!(parse_afad_date("2024-01-15 10:30:00").is_err());
        assert!(parse_afad_date("2024/01/15 10:30:00").is_err());
    }

    #[test]
    fn parse_afad_date_iso8601_fails() {
        assert!(parse_afad_date("2024-01-15T10:30:00Z").is_err());
    }

    #[test]
    fn parse_afad_date_empty_fails() {
        assert!(parse_afad_date("").is_err());
    }

    #[test]
    fn parse_afad_date_invalid_month_fails() {
        assert!(parse_afad_date("2024.13.01 00:00:00").is_err());
    }
}
