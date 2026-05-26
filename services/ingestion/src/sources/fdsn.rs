//! Generic FDSN `fdsnws-event 1.1` source adapter.
//!
//! API reference: https://www.fdsn.org/webservices/FDSN-WS-Specifications-1.2.pdf
//!
//! Supported networks (configured by `FdsnSource::new`):
//! - GFZ  — `https://geofon.gfz-potsdam.de`
//! - INGV — `https://webservices.ingv.it`
//!
//! Query URL shape:
//! ```text
//! {base_url}/fdsnws/event/1/query
//!   ?format=geojson
//!   &starttime={ISO}
//!   &endtime={ISO}
//!   &minmagnitude={f64}
//!   &orderby=time-asc
//!   &limit=1000
//! ```
//!
//! Response: GeoJSON FeatureCollection — same schema as USGS ComCat.
//!
//! Key field shapes:
//! - `feature.properties.code`    → String  — network-local event code
//! - `feature.properties.net`     → String? — network code (e.g. "GFZ")
//! - `feature.properties.time`    → i64     — Unix epoch milliseconds (UTC)
//! - `feature.properties.mag`     → f64?    — magnitude
//! - `feature.properties.magType` → String? — magnitude scale
//! - `feature.properties.place`   → String? — region description
//! - `feature.properties.ids`     → String? — comma-separated authoritative IDs
//! - `geometry.coordinates`       → [lon, lat, depth_km]
//!
//! `source_id` is `{network_code_lowercase}:{event_code}` where `event_code`
//! comes from `properties.code`.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{debug, warn};

use super::SeismicSource;
use crate::config::Config;
use crate::error::{IngestError, ParseError};
use crate::metrics::Metrics;
use crate::schema::RawEarthquakeEvent;
use crate::sources::{normalise_mag_type, validate_coordinates};

// ── Dynamic source config ──────────────────────────────────────────────────────

/// Configuration for a dynamically-configured FDSN source, used when
/// `FDSN_SOURCES` env var is set.
#[derive(Debug, Clone)]
pub struct FdsnSourceConfig {
    pub name: String,
    pub prefix: String,
    pub base_url: String,
}

// ── GeoJSON deserialisation types ─────────────────────────────────────────────

#[derive(Deserialize)]
struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    properties: Properties,
    geometry: Geometry,
}

#[derive(Deserialize)]
struct Properties {
    /// Network-local event code — used to build the canonical `source_id`.
    code: Option<String>,
    /// Network identifier (e.g. "GFZ", "IV"). May differ from the configured
    /// network code; we use the configured code for `source_id` for stability.
    net: Option<String>,
    /// Unix epoch milliseconds (UTC).
    time: Option<i64>,
    mag: Option<f64>,
    #[serde(rename = "magType")]
    mag_type: Option<String>,
    place: Option<String>,
    /// Comma-separated authoritative IDs — stored in raw payload for traceability.
    ids: Option<String>,
}

#[derive(Deserialize)]
struct Geometry {
    /// [longitude, latitude, depth_km]
    coordinates: Vec<f64>,
}

// ── FdsnSource ─────────────────────────────────────────────────────────────────

/// Configuration for a single FDSN network endpoint.
pub struct FdsnSource {
    /// Short label used in logs and metric label values (e.g. `"GFZ"`, `"INGV"`).
    ///
    /// Must be `'static` so it can be returned from `SeismicSource::name()`.
    source_name: &'static str,
    /// Network code in lowercase — used as the prefix in `source_id`.
    network_prefix: &'static str,
    /// Base URL of the FDSN web service, **without** a trailing slash.
    base_url: String,
    client: reqwest::Client,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
}

impl FdsnSource {
    /// Construct a new FDSN source.
    ///
    /// - `source_name`    — short human-readable label (`"GFZ"`, `"INGV"`).
    /// - `network_prefix` — lowercase network code used in `source_id` prefixes
    ///   (`"gfz"`, `"ingv"`).
    /// - `base_url`       — root URL of the fdsnws host, no trailing slash.
    pub fn new(
        source_name: &'static str,
        network_prefix: &'static str,
        base_url: impl Into<String>,
        client: reqwest::Client,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            source_name,
            network_prefix,
            base_url: base_url.into(),
            client,
            config,
            metrics,
        }
    }

    /// Construct a new FDSN source from a [`FdsnSourceConfig`].
    ///
    /// The `name` and `prefix` strings from the config are leaked onto the
    /// heap so they satisfy the `&'static str` requirement of `SeismicSource::name()`.
    /// This is acceptable because FDSN sources live for the lifetime of the
    /// process.
    pub fn from_config(
        cfg: FdsnSourceConfig,
        client: reqwest::Client,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> Self {
        // SAFETY: Both strings are leaked intentionally — FDSN source configs
        // are created once at startup and must outlive the entire process.
        // The memory is never freed, which is acceptable for a small, bounded
        // set of config values whose count equals the number of FDSN networks.
        let source_name: &'static str = Box::leak(cfg.name.into_boxed_str());
        let network_prefix: &'static str = Box::leak(cfg.prefix.into_boxed_str());
        Self {
            source_name,
            network_prefix,
            base_url: cfg.base_url,
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
        let url = format!(
            "{}/fdsnws/event/1/query\
             ?format=geojson\
             &starttime={}\
             &endtime={}\
             &minmagnitude={}\
             &orderby=time-asc\
             &limit=1000",
            self.base_url,
            since.format("%Y-%m-%dT%H:%M:%S"),
            now.format("%Y-%m-%dT%H:%M:%S"),
            self.config.min_magnitude,
        );

        debug!(url = %url, source = self.source_name, "Fetching FDSN events");

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| IngestError::HttpFetch {
                src: self.source_name,
                inner: e,
            })?;

        let status = response.status();

        // HTTP 204 is the canonical FDSN "no events matched" response.
        if status == reqwest::StatusCode::NO_CONTENT {
            debug!(source = self.source_name, "No events in window (HTTP 204)");
            return Ok(vec![]);
        }

        let response = response
            .error_for_status()
            .map_err(|e| IngestError::HttpFetch {
                src: self.source_name,
                inner: e,
            })?;

        let body = response.bytes().await.map_err(|e| IngestError::HttpFetch {
            src: self.source_name,
            inner: e,
        })?;

        // Some FDSN implementations return HTTP 200 with an empty body when
        // there are no events in the requested window (non-standard but seen in
        // practice). Treat it the same as HTTP 204.
        if body.is_empty() {
            debug!(
                source = self.source_name,
                "No events in window (HTTP 200 + empty body)"
            );
            return Ok(vec![]);
        }

        let collection: FeatureCollection =
            serde_json::from_slice(&body).map_err(|e| IngestError::JsonParse {
                src: self.source_name,
                event_id: "(collection)".into(),
                inner: e,
            })?;

        let ingested_at_ms = Utc::now().timestamp_millis();
        let api_count = collection.features.len();
        let mut events = Vec::with_capacity(api_count);

        for feature in collection.features {
            match parse_feature(
                feature,
                self.source_name,
                self.network_prefix,
                ingested_at_ms,
                &self.config.pipeline_version,
            ) {
                Ok(event) => events.push(event),
                Err(e) => {
                    warn!(
                        error = %e,
                        source = self.source_name,
                        "Skipping unparseable event"
                    );
                    self.metrics
                        .events_rejected_total
                        .with_label_values(&[self.source_name, "parse_error"])
                        .inc();
                }
            }
        }

        if api_count >= 1000 {
            warn!(
                source = self.source_name,
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
impl SeismicSource for FdsnSource {
    fn name(&self) -> &'static str {
        self.source_name
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

// ── Feature parser ─────────────────────────────────────────────────────────────

fn parse_feature(
    feature: Feature,
    source_name: &'static str,
    network_prefix: &str,
    ingested_at_ms: i64,
    pipeline_version: &str,
) -> Result<RawEarthquakeEvent, ParseError> {
    let props = &feature.properties;

    // The `code` property is the network-local event identifier and is
    // required to construct a stable `source_id`.
    let event_code = props
        .code
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ParseError::MissingField {
            field: "properties.code",
            src: source_name,
            event_id: "(unknown)".into(),
        })?;

    let event_id = event_code.to_owned();

    let magnitude = props.mag.ok_or_else(|| ParseError::MissingField {
        field: "mag",
        src: source_name,
        event_id: event_id.clone(),
    })?;
    if !magnitude.is_finite() {
        return Err(ParseError::InvalidField {
            field: "magnitude",
            src: source_name,
            event_id: event_id.clone(),
            detail: format!("{magnitude} is not a finite magnitude"),
        });
    }

    let event_time_ms = props.time.ok_or_else(|| ParseError::MissingField {
        field: "time",
        src: source_name,
        event_id: event_id.clone(),
    })?;

    let coords = &feature.geometry.coordinates;
    if coords.len() < 2 {
        return Err(ParseError::InvalidField {
            field: "geometry.coordinates",
            src: source_name,
            event_id: event_id.clone(),
            detail: format!("expected at least 2 elements, got {}", coords.len()),
        });
    }
    let longitude = coords[0];
    let latitude = coords[1];
    let depth_km = coords.get(2).copied().filter(|d| d.is_finite());

    validate_coordinates(latitude, longitude, source_name, &event_id)?;

    let magnitude_type = normalise_mag_type(props.mag_type.as_deref().unwrap_or("UNKNOWN"));

    // FDSN networks are reviewed by the issuing agency; quality is "C"
    // (official agency, automated solution) as there is no review-status
    // field in the standard GeoJSON response.
    let quality_indicator = "C".to_owned();

    let source_id = format!("{}:{}", network_prefix, event_code);

    let raw_payload = serde_json::json!({
        "code": event_code,
        "net": props.net,
        "ids": props.ids,
        "time": event_time_ms,
        "mag": magnitude,
        "magType": magnitude_type,
        "place": props.place,
        "coordinates": coords,
    })
    .to_string();

    Ok(RawEarthquakeEvent {
        source_id,
        source_network: source_name.to_owned(),
        event_time_ms,
        latitude,
        longitude,
        depth_km,
        magnitude,
        magnitude_type,
        region_name: props.place.clone(),
        quality_indicator,
        raw_payload,
        ingested_at_ms,
        pipeline_version: pipeline_version.to_owned(),
    })
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn make_feature(
        code: Option<&str>,
        time: Option<i64>,
        mag: Option<f64>,
        mag_type: Option<&str>,
        lat: f64,
        lon: f64,
        depth: Option<f64>,
        place: Option<&str>,
    ) -> Feature {
        let mut coords = vec![lon, lat];
        if let Some(d) = depth {
            coords.push(d);
        }
        Feature {
            properties: Properties {
                code: code.map(str::to_owned),
                net: Some("GFZ".into()),
                time,
                mag: Some(mag.unwrap_or(3.0)),
                mag_type: mag_type.map(str::to_owned),
                place: place.map(str::to_owned),
                ids: None,
            },
            geometry: Geometry {
                coordinates: coords,
            },
        }
    }

    fn valid_feature() -> Feature {
        make_feature(
            Some("gfz2024abcd"),
            Some(1_704_067_200_000),
            Some(3.5),
            Some("ML"),
            38.0,
            27.5,
            Some(10.0),
            Some("Central Turkey"),
        )
    }

    #[test]
    fn parse_feature_valid_full() {
        let f = valid_feature();
        let event = parse_feature(f, "GFZ", "gfz", 0, "0.3.0").unwrap();
        assert_eq!(event.source_id, "gfz:gfz2024abcd");
        assert_eq!(event.source_network, "GFZ");
        assert!((event.magnitude - 3.5).abs() < 1e-9);
        assert_eq!(event.magnitude_type, "ML");
        assert!((event.latitude - 38.0).abs() < 1e-9);
        assert!((event.longitude - 27.5).abs() < 1e-9);
        assert_eq!(event.depth_km, Some(10.0));
        assert_eq!(event.region_name.as_deref(), Some("Central Turkey"));
        assert_eq!(event.quality_indicator, "C");
        assert_eq!(event.pipeline_version, "0.3.0");
        assert_eq!(event.event_time_ms, 1_704_067_200_000);
    }

    #[test]
    fn parse_feature_ingv_source_id_prefix() {
        let mut f = valid_feature();
        f.properties.code = Some("ingv2024xyz".into());
        let event = parse_feature(f, "INGV", "ingv", 0, "0.3.0").unwrap();
        assert_eq!(event.source_id, "ingv:ingv2024xyz");
        assert_eq!(event.source_network, "INGV");
    }

    #[test]
    fn parse_feature_missing_code_fails() {
        let mut f = valid_feature();
        f.properties.code = None;
        assert!(parse_feature(f, "GFZ", "gfz", 0, "0.3.0").is_err());
    }

    #[test]
    fn parse_feature_empty_code_fails() {
        let mut f = valid_feature();
        f.properties.code = Some(String::new());
        assert!(parse_feature(f, "GFZ", "gfz", 0, "0.3.0").is_err());
    }

    #[test]
    fn parse_feature_missing_mag_fails() {
        let mut f = valid_feature();
        f.properties.mag = None;
        assert!(parse_feature(f, "GFZ", "gfz", 0, "0.3.0").is_err());
    }

    #[test]
    fn parse_feature_nan_mag_fails() {
        let mut f = valid_feature();
        f.properties.mag = Some(f64::NAN);
        assert!(parse_feature(f, "GFZ", "gfz", 0, "0.3.0").is_err());
    }

    #[test]
    fn parse_feature_missing_time_fails() {
        let mut f = valid_feature();
        f.properties.time = None;
        assert!(parse_feature(f, "GFZ", "gfz", 0, "0.3.0").is_err());
    }

    #[test]
    fn parse_feature_too_few_coordinates_fails() {
        let mut f = valid_feature();
        f.geometry.coordinates = vec![27.5]; // only longitude
        assert!(parse_feature(f, "GFZ", "gfz", 0, "0.3.0").is_err());
    }

    #[test]
    fn parse_feature_invalid_lat_fails() {
        let mut f = valid_feature();
        f.geometry.coordinates = vec![27.5, 91.0, 10.0]; // lat > 90
        assert!(parse_feature(f, "GFZ", "gfz", 0, "0.3.0").is_err());
    }

    #[test]
    fn parse_feature_depth_missing_yields_none() {
        let f = make_feature(
            Some("gfz2024abcd"),
            Some(1_704_067_200_000),
            Some(3.5),
            Some("ML"),
            38.0,
            27.5,
            None,
            None,
        );
        let event = parse_feature(f, "GFZ", "gfz", 0, "0.3.0").unwrap();
        assert_eq!(event.depth_km, None);
    }

    #[test]
    fn parse_feature_unknown_mag_type_normalised() {
        let mut f = valid_feature();
        f.properties.mag_type = None;
        let event = parse_feature(f, "GFZ", "gfz", 0, "0.3.0").unwrap();
        assert_eq!(event.magnitude_type, "UNKNOWN");
    }

    #[test]
    fn parse_feature_mag_type_lowercased_normalised() {
        let mut f = valid_feature();
        f.properties.mag_type = Some("mw".into());
        let event = parse_feature(f, "GFZ", "gfz", 0, "0.3.0").unwrap();
        assert_eq!(event.magnitude_type, "MW");
    }

    #[test]
    fn parse_feature_raw_payload_contains_key_fields() {
        let event = parse_feature(valid_feature(), "GFZ", "gfz", 0, "0.3.0").unwrap();
        let payload: serde_json::Value = serde_json::from_str(&event.raw_payload).unwrap();
        assert_eq!(payload["code"], serde_json::json!("gfz2024abcd"));
        assert_eq!(payload["mag"], serde_json::json!(3.5));
        assert_eq!(payload["time"], serde_json::json!(1_704_067_200_000_i64));
    }
}
