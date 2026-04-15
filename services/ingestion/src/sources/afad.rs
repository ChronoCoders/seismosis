//! AFAD (Afet ve Acil Durum Yönetimi Başkanlığı) earthquake source.
//!
//! API endpoint: https://deprem.afad.gov.tr/apiv2/event/filter
//!
//! The API returns a JSON array of earthquake objects. Time range is passed
//! via `start` and `end` query parameters in "YYYY-MM-DD HH:MM:SS" format
//! (space-separated, UTC, dashes as date separator).
//!
//! Key field shapes (verified against live API 2026-04-12):
//! - `eventID`       → String  — AFAD numeric identifier (e.g. "713106")
//! - `date`          → String  — ISO-8601 UTC without timezone suffix
//!   (e.g. "2026-04-11T00:19:09"). Treat as UTC.
//! - `magnitude`     → String  — decimal magnitude (e.g. "1.3")
//! - `type`          → String  — magnitude scale (e.g. "ML", "Mw"). Note: this
//!   is the magnitude type, NOT an event classification.
//! - `latitude`      → String  — decimal degrees (e.g. "37.90472")
//! - `longitude`     → String  — decimal degrees (e.g. "36.45944")
//! - `depth`         → String  — depth in km, positive downward (e.g. "6.94")
//! - `location`      → String? — Turkish region/district description
//! - `isEventUpdate` → bool?   — true if this record supersedes an earlier one
//!
//! NOTE: All numeric fields are returned as strings. Each is parsed explicitly;
//! a parse failure skips the event with a warning log and a metric increment.
//!
//! NOTE: `date` carries no timezone suffix. The AFAD v2 API consistently
//! returns UTC values despite the public website displaying UTC+3.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::Deserialize;
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{debug, warn};

use super::SeismicSource;
use crate::config::Config;
use crate::error::{IngestError, ParseError};
use crate::metrics::Metrics;
use crate::schema::RawEarthquakeEvent;
use crate::sources::{afad_quality, normalise_mag_type, validate_coordinates};

const SOURCE_NAME: &str = "AFAD";

// ─── Serde shapes ────────────────────────────────────────────────────────────

/// A single earthquake entry in the AFAD v2 response array.
///
/// All numeric fields are delivered as strings; `Option<String>` allows
/// future API changes to add or remove fields without breaking deserialization.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AfadEvent {
    #[serde(rename = "eventID")]
    event_id: Option<String>,
    /// ISO-8601 UTC datetime without timezone suffix (e.g. "2026-04-11T00:19:09").
    date: Option<String>,
    /// Decimal magnitude as a string (e.g. "3.2").
    magnitude: Option<String>,
    /// Magnitude scale (e.g. "ML", "Mw"). This field is named `type` in the
    /// wire format — it is the magnitude type, not an event classification.
    #[serde(rename = "type")]
    magnitude_type_raw: Option<String>,
    /// Decimal latitude as a string.
    latitude: Option<String>,
    /// Decimal longitude as a string.
    longitude: Option<String>,
    /// Depth in km, positive downward, as a string.
    depth: Option<String>,
    /// Turkish region/district description.
    location: Option<String>,
    /// True if this record supersedes an earlier automated solution.
    is_event_update: Option<bool>,
}

// ─── Source implementation ────────────────────────────────────────────────────

pub struct AfadSource {
    client: reqwest::Client,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
}

impl AfadSource {
    pub fn new(client: reqwest::Client, config: Arc<Config>, metrics: Arc<Metrics>) -> Self {
        Self {
            client,
            config,
            metrics,
        }
    }

    async fn fetch_once(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<RawEarthquakeEvent>, IngestError> {
        let now = Utc::now();

        // AFAD v2 expects space-separated datetime in "YYYY-MM-DD HH:MM:SS" format.
        // reqwest percent-encodes the space as %20 when building the URL.
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
            .map_err(|e| IngestError::HttpFetch {
                src: SOURCE_NAME,
                inner: e,
            })?;

        // error_for_status() consumes `response` and returns Ok(response) for
        // 2xx and Err(reqwest::Error) for non-2xx — no unwrap/expect needed.
        let response = response
            .error_for_status()
            .map_err(|e| IngestError::HttpFetch {
                src: SOURCE_NAME,
                inner: e,
            })?;

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
                    self.metrics
                        .events_rejected_total
                        .with_label_values(&[SOURCE_NAME, "parse_error"])
                        .inc();
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
    let event_id = raw
        .event_id
        .clone()
        .ok_or_else(|| ParseError::MissingField {
            field: "eventID",
            src: SOURCE_NAME,
            event_id: "(unknown)".into(),
        })?;

    // ── date ─────────────────────────────────────────────────────────────────
    let date_str = raw
        .date
        .as_deref()
        .ok_or_else(|| ParseError::MissingField {
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

    // ── magnitude ─────────────────────────────────────────────────────────────
    let mag_str = raw
        .magnitude
        .as_deref()
        .ok_or_else(|| ParseError::MissingField {
            field: "magnitude",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
        })?;
    let magnitude = mag_str
        .trim()
        .parse::<f64>()
        .map_err(|_| ParseError::InvalidField {
            field: "magnitude",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
            detail: format!("cannot parse '{}' as f64", mag_str),
        })?;

    // ── coordinates ──────────────────────────────────────────────────────────
    let lat_str = raw
        .latitude
        .as_deref()
        .ok_or_else(|| ParseError::MissingField {
            field: "latitude",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
        })?;
    let latitude = lat_str
        .trim()
        .parse::<f64>()
        .map_err(|_| ParseError::InvalidField {
            field: "latitude",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
            detail: format!("cannot parse '{}' as f64", lat_str),
        })?;

    let lon_str = raw
        .longitude
        .as_deref()
        .ok_or_else(|| ParseError::MissingField {
            field: "longitude",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
        })?;
    let longitude = lon_str
        .trim()
        .parse::<f64>()
        .map_err(|_| ParseError::InvalidField {
            field: "longitude",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
            detail: format!("cannot parse '{}' as f64", lon_str),
        })?;

    validate_coordinates(latitude, longitude, SOURCE_NAME, &event_id)?;

    // ── depth ─────────────────────────────────────────────────────────────────
    // Missing or non-finite → None (stored as NULL). Negative values are
    // physically invalid for this field; log and discard rather than silently
    // promote to NULL without a trace.
    let depth_km = match raw.depth.as_deref() {
        None => None,
        Some(s) => match s.trim().parse::<f64>() {
            Ok(d) if !d.is_finite() => None,
            Ok(d) if d < 0.0 => {
                warn!(
                    source = SOURCE_NAME,
                    event_id = %event_id,
                    depth = d,
                    "Discarding negative depth value — treating as unknown",
                );
                None
            }
            Ok(d) => Some(d),
            Err(_) => None,
        },
    };

    // ── magnitude type ────────────────────────────────────────────────────────
    // The `type` field in the AFAD v2 API is the magnitude scale, not an event
    // classification. There is no review-status field in this API version.
    let magnitude_type = normalise_mag_type(raw.magnitude_type_raw.as_deref().unwrap_or("UNKNOWN"));

    let quality_indicator = afad_quality(raw.is_event_update).to_owned();
    let source_id = format!("AFAD:{}", event_id);

    // Store original wire strings so the payload is a faithful forensic record.
    let raw_payload = serde_json::json!({
        "eventID": event_id,
        "date": date_str,
        "magnitude": mag_str,
        "type": raw.magnitude_type_raw,
        "latitude": lat_str,
        "longitude": lon_str,
        "depth": raw.depth,
        "location": raw.location,
        "isEventUpdate": raw.is_event_update,
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

/// Parse an AFAD v2 UTC datetime string into Unix epoch milliseconds.
///
/// The live API returns ISO-8601-like strings without a timezone suffix, e.g.
/// `"2026-04-11T00:19:09"`. These are treated as UTC per AFAD v2 API documentation.
/// A trailing `Z` is accepted but not required.
fn parse_afad_date(s: &str) -> Result<i64, String> {
    let s = s.trim();

    // Accept with or without trailing Z.
    let naive = if let Some(stripped) = s.strip_suffix('Z') {
        NaiveDateTime::parse_from_str(stripped, "%Y-%m-%dT%H:%M:%S")
    } else {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
    };

    naive
        .map(|ndt| ndt.and_utc().timestamp_millis())
        .map_err(|e| {
            format!(
                "'{}' does not match AFAD date format YYYY-MM-DDTHH:MM:SS: {}",
                s, e
            )
        })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_afad_date ───────────────────────────────────────────────────────

    #[test]
    fn parse_afad_date_valid_no_z() {
        // 2026-04-11T00:19:09 UTC
        let ms = parse_afad_date("2026-04-11T00:19:09").unwrap();
        assert_eq!(ms, 1_744_330_749_000);
    }

    #[test]
    fn parse_afad_date_valid_with_z() {
        let ms_z = parse_afad_date("2026-04-11T00:19:09Z").unwrap();
        let ms_no_z = parse_afad_date("2026-04-11T00:19:09").unwrap();
        assert_eq!(ms_z, ms_no_z);
    }

    #[test]
    fn parse_afad_date_epoch() {
        let ms = parse_afad_date("1970-01-01T00:00:00").unwrap();
        assert_eq!(ms, 0);
    }

    #[test]
    fn parse_afad_date_leading_trailing_whitespace() {
        let ms = parse_afad_date("  2026-04-11T00:19:09  ").unwrap();
        assert_eq!(ms, 1_744_330_749_000);
    }

    #[test]
    fn parse_afad_date_old_dot_format_fails() {
        // The old (incorrect) YYYY.MM.DD format must not be accepted.
        assert!(parse_afad_date("2026.04.11 00:19:09").is_err());
    }

    #[test]
    fn parse_afad_date_empty_fails() {
        assert!(parse_afad_date("").is_err());
    }

    #[test]
    fn parse_afad_date_invalid_month_fails() {
        assert!(parse_afad_date("2026-13-01T00:00:00").is_err());
    }

    // ── parse_event ───────────────────────────────────────────────────────────

    fn valid_raw() -> AfadEvent {
        AfadEvent {
            event_id: Some("713106".into()),
            date: Some("2026-04-11T00:19:09".into()),
            magnitude: Some("3.2".into()),
            magnitude_type_raw: Some("ML".into()),
            latitude: Some("37.90472".into()),
            longitude: Some("36.45944".into()),
            depth: Some("6.94".into()),
            location: Some("Göksun (Kahramanmaraş)".into()),
            is_event_update: Some(false),
        }
    }

    #[test]
    fn parse_event_valid_full() {
        let event = parse_event(valid_raw(), 0, "0.1.0").unwrap();
        assert_eq!(event.source_id, "AFAD:713106");
        assert_eq!(event.source_network, "AFAD");
        assert!((event.magnitude - 3.2).abs() < 1e-9);
        assert_eq!(event.magnitude_type, "ML");
        assert!((event.latitude - 37.90472).abs() < 1e-6);
        assert!((event.longitude - 36.45944).abs() < 1e-6);
        assert_eq!(event.depth_km, Some(6.94));
        assert_eq!(event.region_name.as_deref(), Some("Göksun (Kahramanmaraş)"));
        assert_eq!(event.quality_indicator, "C"); // isEventUpdate=false → "C"
        assert_eq!(event.pipeline_version, "0.1.0");
    }

    #[test]
    fn parse_event_is_event_update_true_gives_quality_b() {
        let mut raw = valid_raw();
        raw.is_event_update = Some(true);
        let event = parse_event(raw, 0, "0.1.0").unwrap();
        assert_eq!(event.quality_indicator, "B");
    }

    #[test]
    fn parse_event_is_event_update_none_gives_quality_c() {
        let mut raw = valid_raw();
        raw.is_event_update = None;
        let event = parse_event(raw, 0, "0.1.0").unwrap();
        assert_eq!(event.quality_indicator, "C");
    }

    #[test]
    fn parse_event_missing_event_id_fails() {
        let mut raw = valid_raw();
        raw.event_id = None;
        assert!(parse_event(raw, 0, "0.1.0").is_err());
    }

    #[test]
    fn parse_event_missing_magnitude_fails() {
        let mut raw = valid_raw();
        raw.magnitude = None;
        let err = parse_event(raw, 0, "0.1.0").unwrap_err();
        assert!(err.to_string().contains("magnitude"));
    }

    #[test]
    fn parse_event_invalid_magnitude_float_fails() {
        let mut raw = valid_raw();
        raw.magnitude = Some("not_a_number".into());
        let err = parse_event(raw, 0, "0.1.0").unwrap_err();
        assert!(err.to_string().contains("magnitude"));
    }

    #[test]
    fn parse_event_missing_latitude_fails() {
        let mut raw = valid_raw();
        raw.latitude = None;
        let err = parse_event(raw, 0, "0.1.0").unwrap_err();
        assert!(err.to_string().contains("latitude"));
    }

    #[test]
    fn parse_event_nan_latitude_string_fails() {
        // "NaN" parses as f64::NAN, which fails validate_coordinates.
        let mut raw = valid_raw();
        raw.latitude = Some("NaN".into());
        assert!(parse_event(raw, 0, "0.1.0").is_err());
    }

    #[test]
    fn parse_event_nan_longitude_string_fails() {
        let mut raw = valid_raw();
        raw.longitude = Some("NaN".into());
        assert!(parse_event(raw, 0, "0.1.0").is_err());
    }

    #[test]
    fn parse_event_negative_depth_yields_none_depth() {
        let mut raw = valid_raw();
        raw.depth = Some("-5.0".into());
        // parse_event succeeds but depth_km is None (negative depth discarded).
        let event = parse_event(raw, 0, "0.1.0").unwrap();
        assert_eq!(event.depth_km, None);
    }

    #[test]
    fn parse_event_missing_depth_yields_none_depth() {
        let mut raw = valid_raw();
        raw.depth = None;
        let event = parse_event(raw, 0, "0.1.0").unwrap();
        assert_eq!(event.depth_km, None);
    }

    #[test]
    fn parse_event_unknown_magnitude_type_normalised() {
        let mut raw = valid_raw();
        raw.magnitude_type_raw = None;
        let event = parse_event(raw, 0, "0.1.0").unwrap();
        assert_eq!(event.magnitude_type, "UNKNOWN");
    }

    #[test]
    fn parse_event_raw_payload_stores_original_strings() {
        let event = parse_event(valid_raw(), 0, "0.1.0").unwrap();
        let payload: serde_json::Value = serde_json::from_str(&event.raw_payload).unwrap();
        // Magnitude must be stored as the original string, not a parsed float.
        assert_eq!(
            payload["magnitude"],
            serde_json::Value::String("3.2".into())
        );
        assert_eq!(
            payload["latitude"],
            serde_json::Value::String("37.90472".into())
        );
        assert_eq!(
            payload["date"],
            serde_json::Value::String("2026-04-11T00:19:09".into())
        );
    }
}
