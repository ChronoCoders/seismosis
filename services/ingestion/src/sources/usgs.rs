//! USGS FDSNWS earthquake source.
//!
//! API docs: https://earthquake.usgs.gov/fdsnws/event/1/
//! Response: GeoJSON FeatureCollection.
//!
//! Key field shapes (confirmed against live API 2024-01-01):
//! - `feature.id`            → String   (e.g. "us6000m0wb") — canonical event ID
//! - `properties.time`       → i64      — Unix epoch milliseconds (UTC)
//! - `properties.mag`        → f64      — magnitude
//! - `properties.magType`    → String   — magnitude scale (lowercase from USGS)
//! - `properties.place`      → String?  — region description
//! - `properties.status`     → String?  — "reviewed" | "automatic"
//! - `properties.type`       → String?  — "earthquake" | "quarry blast" | ...
//! - `geometry.coordinates`  → [lon, lat, depth_km]

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{debug, warn};

use crate::config::Config;
use crate::error::{IngestError, ParseError};
use crate::schema::RawEarthquakeEvent;
use crate::sources::{normalise_mag_type, usgs_quality, validate_coordinates};
use super::SeismicSource;

const SOURCE_NAME: &str = "USGS";

// ─── Serde shapes ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FeatureCollection {
    features: Vec<Feature>,
}

#[derive(Deserialize)]
struct Feature {
    id: String,
    properties: Properties,
    geometry: Geometry,
}

#[derive(Deserialize)]
struct Properties {
    mag: Option<f64>,
    place: Option<String>,
    time: Option<i64>,
    status: Option<String>,
    #[serde(rename = "magType")]
    mag_type: Option<String>,
    #[serde(rename = "type")]
    event_type: Option<String>,
}

#[derive(Deserialize)]
struct Geometry {
    /// [longitude, latitude, depth_km]
    coordinates: Vec<f64>,
}

// ─── Source implementation ────────────────────────────────────────────────────

pub struct UsgsSource {
    client: reqwest::Client,
    config: Arc<Config>,
}

impl UsgsSource {
    pub fn new(client: reqwest::Client, config: Arc<Config>) -> Self {
        Self { client, config }
    }

    async fn fetch_once(
        &self,
        since: DateTime<Utc>,
    ) -> Result<Vec<RawEarthquakeEvent>, IngestError> {
        let now = Utc::now();
        let url = format!(
            "{}/query?format=geojson\
             &starttime={}\
             &endtime={}\
             &minmagnitude={}\
             &orderby=time\
             &limit=1000",
            self.config.usgs_api_url,
            since.format("%Y-%m-%dT%H:%M:%S"),
            now.format("%Y-%m-%dT%H:%M:%S"),
            self.config.min_magnitude,
        );

        debug!(url = %url, "Fetching USGS events");

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
            match parse_feature(
                feature,
                ingested_at_ms,
                &self.config.pipeline_version,
            ) {
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
impl SeismicSource for UsgsSource {
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

    // Filter: only process earthquakes (not quarry blasts, explosions, etc.)
    if let Some(ref t) = feature.properties.event_type {
        if t != "earthquake" {
            return Err(ParseError::Filtered {
                reason: "non_earthquake",
                src: SOURCE_NAME,
                event_id,
            });
        }
    }

    let magnitude = feature
        .properties
        .mag
        .ok_or_else(|| ParseError::MissingField {
            field: "mag",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
        })?;

    let event_time_ms =
        feature
            .properties
            .time
            .ok_or_else(|| ParseError::MissingField {
                field: "time",
                src: SOURCE_NAME,
                event_id: event_id.clone(),
            })?;

    // geometry.coordinates = [lon, lat, depth_km]
    let coords = &feature.geometry.coordinates;
    if coords.len() < 2 {
        return Err(ParseError::InvalidField {
            field: "geometry.coordinates",
            src: SOURCE_NAME,
            event_id: event_id.clone(),
            detail: format!("expected at least 2 elements, got {}", coords.len()),
        });
    }
    let longitude = coords[0];
    let latitude = coords[1];
    let depth_km = coords.get(2).copied().filter(|d| d.is_finite());

    validate_coordinates(latitude, longitude, SOURCE_NAME, &event_id)?;

    let magnitude_type = normalise_mag_type(
        feature
            .properties
            .mag_type
            .as_deref()
            .unwrap_or("UNKNOWN"),
    );

    let quality_indicator = usgs_quality(feature.properties.status.as_deref()).to_owned();
    let source_id = format!("USGS:{}", event_id);

    // Serialise the original properties to JSON for the raw_payload field.
    let raw_payload = serde_json::json!({
        "id": feature.id,
        "mag": magnitude,
        "magType": magnitude_type,
        "place": feature.properties.place,
        "time": event_time_ms,
        "status": feature.properties.status,
        "type": feature.properties.event_type,
        "coordinates": coords,
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
        region_name: feature.properties.place,
        quality_indicator,
        raw_payload,
        ingested_at_ms,
        pipeline_version: pipeline_version.to_owned(),
    })
}
