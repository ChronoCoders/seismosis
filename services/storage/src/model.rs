use chrono::{DateTime, TimeZone, Utc};

use crate::error::ProcessError;

/// Decoded and validated earthquake event ready for the PostgreSQL upsert.
///
/// All fields map directly to columns in `seismology.seismic_events`.
#[derive(Debug)]
pub struct EarthquakeEvent {
    pub source_id: String,
    pub source_network: String,
    pub event_time: DateTime<Utc>,
    /// WGS-84, [-90, 90].
    pub latitude: f64,
    /// WGS-84, [-180, 180].
    pub longitude: f64,
    /// Positive = deeper below surface.
    pub depth_km: Option<f64>,
    /// Validated to be in [-2.0, 10.0] (DB CHECK constraint).
    pub magnitude: f64,
    pub magnitude_type: String,
    pub region_name: Option<String>,
    /// Single character: A / B / C / D (DB CHECK constraint).
    pub quality_indicator: String,
    /// Original source JSON parsed from the Avro `raw_payload` string field.
    pub raw_payload: serde_json::Value,
    /// Wall-clock time when the ingestion service first received this event.
    pub processed_at: DateTime<Utc>,
    pub pipeline_version: String,
}

/// Raw field values decoded from the Avro record, before validation and
/// type conversion.
#[derive(Debug)]
pub struct RawFields {
    pub source_id: String,
    pub source_network: String,
    pub event_time_ms: i64,
    pub latitude: f64,
    pub longitude: f64,
    pub depth_km: Option<f64>,
    pub magnitude: f64,
    pub magnitude_type: String,
    pub region_name: Option<String>,
    pub quality_indicator: String,
    pub raw_payload: String,
    pub ingested_at_ms: i64,
    pub pipeline_version: String,
}

impl RawFields {
    /// Validate all fields and produce a type-converted `EarthquakeEvent`.
    ///
    /// Checks are aligned with the DB constraints in `seismology.seismic_events`
    /// so that no row is rejected by a CHECK constraint after reaching the DB.
    pub fn validate(self) -> Result<EarthquakeEvent, ProcessError> {
        if self.source_id.is_empty() {
            return Err(ProcessError::Validation {
                field: "source_id",
                detail: "must not be empty".to_owned(),
            });
        }

        // source_network — aligns with DB column VARCHAR(50).
        if self.source_network.len() > 50 {
            return Err(ProcessError::Validation {
                field: "source_network",
                detail: format!(
                    "length {} exceeds VARCHAR(50) limit",
                    self.source_network.len()
                ),
            });
        }

        // magnitude_type — aligns with DB column VARCHAR(10).
        if self.magnitude_type.len() > 10 {
            return Err(ProcessError::Validation {
                field: "magnitude_type",
                detail: format!(
                    "length {} exceeds VARCHAR(10) limit",
                    self.magnitude_type.len()
                ),
            });
        }

        // Coordinates — guard is_finite before range to avoid NaN comparisons.
        if !self.latitude.is_finite() || !(-90.0..=90.0).contains(&self.latitude) {
            return Err(ProcessError::Validation {
                field: "latitude",
                detail: format!("{} is not in [-90, 90]", self.latitude),
            });
        }
        if !self.longitude.is_finite() || !(-180.0..=180.0).contains(&self.longitude) {
            return Err(ProcessError::Validation {
                field: "longitude",
                detail: format!("{} is not in [-180, 180]", self.longitude),
            });
        }

        // depth_km — aligns with DB CHECK (depth_km BETWEEN -5 AND 800).
        if let Some(d) = self.depth_km {
            if !d.is_finite() || !(-5.0..=800.0).contains(&d) {
                return Err(ProcessError::Validation {
                    field: "depth_km",
                    detail: format!("{} is not in [-5, 800]", d),
                });
            }
        }

        // magnitude — aligns with DB CHECK (magnitude BETWEEN -2.0 AND 10.0).
        if !self.magnitude.is_finite() || !(-2.0..=10.0).contains(&self.magnitude) {
            return Err(ProcessError::Validation {
                field: "magnitude",
                detail: format!("{} is not in [-2.0, 10.0]", self.magnitude),
            });
        }

        // quality_indicator — aligns with DB CHECK IN ('A','B','C','D').
        if !matches!(self.quality_indicator.as_str(), "A" | "B" | "C" | "D") {
            return Err(ProcessError::Validation {
                field: "quality_indicator",
                detail: format!("'{}' is not one of A, B, C, D", self.quality_indicator),
            });
        }

        // Timestamps.
        let event_time = ms_to_datetime(self.event_time_ms).ok_or(ProcessError::Validation {
            field: "event_time_ms",
            detail: format!("{} overflows DateTime<Utc>", self.event_time_ms),
        })?;
        let processed_at = ms_to_datetime(self.ingested_at_ms).ok_or(ProcessError::Validation {
            field: "ingested_at_ms",
            detail: format!("{} overflows DateTime<Utc>", self.ingested_at_ms),
        })?;

        // raw_payload must be parseable JSON (column type is JSONB).
        let raw_payload: serde_json::Value = serde_json::from_str(&self.raw_payload)
            .map_err(|e| ProcessError::InvalidPayloadJson(e.to_string()))?;

        Ok(EarthquakeEvent {
            source_id: self.source_id,
            source_network: self.source_network,
            event_time,
            latitude: self.latitude,
            longitude: self.longitude,
            depth_km: self.depth_km,
            magnitude: self.magnitude,
            magnitude_type: self.magnitude_type,
            region_name: self.region_name,
            quality_indicator: self.quality_indicator,
            raw_payload,
            processed_at,
            pipeline_version: self.pipeline_version,
        })
    }
}

fn ms_to_datetime(ms: i64) -> Option<DateTime<Utc>> {
    let secs = ms.div_euclid(1_000);
    // rem_euclid(1_000) is always in [0, 999]; * 1_000_000 ≤ 999_000_000 < u32::MAX.
    let nanos = (ms.rem_euclid(1_000) * 1_000_000) as u32;
    Utc.timestamp_opt(secs, nanos).single()
}
