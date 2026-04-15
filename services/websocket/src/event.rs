//! Domain models for events consumed from Redpanda and pushed to WebSocket clients.
//!
//! Both event types derive `serde::Deserialize` so they can be built from
//! `apache_avro::from_value`.  They also derive `serde::Serialize` so they can
//! be serialised to JSON for the wire format to WebSocket clients.
//!
//! ## Wire format to clients
//!
//! Each message is a JSON object with a `"type"` discriminant field:
//!
//! ```json
//! {"type":"earthquake","source_id":"USGS:us7000xyz","ml_magnitude":5.18,...}
//! {"type":"alert","source_id":"USGS:us7000xyz","alert_level":"ORANGE",...}
//! ```
//!
//! The `"type"` field is injected by `ServerMessage`'s internally-tagged
//! serialisation — callers do not need to add it manually.

use serde::{Deserialize, Serialize};

// ─── Enriched earthquake event ────────────────────────────────────────────────

/// Decoded from the `earthquakes.enriched` Avro topic.
///
/// Field names match the Avro schema exactly (`earthquakes.enriched-value`)
/// so that `apache_avro::from_value::<EnrichedEvent>` works without renames.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EnrichedEvent {
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
    // The raw_payload field is omitted from the WebSocket output — it is
    // large, not useful to browser clients, and may contain raw source data.
    // It is deserialized from Avro but never read; the allow is intentional.
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    pub raw_payload: String,
    pub ingested_at_ms: i64,
    pub pipeline_version: String,
    pub ml_magnitude: f64,
    pub ml_magnitude_source: String,
    pub is_aftershock: bool,
    pub mainshock_source_id: Option<String>,
    pub mainshock_magnitude: Option<f64>,
    pub mainshock_distance_km: Option<f64>,
    pub mainshock_time_delta_hours: Option<f64>,
    pub estimated_felt_radius_km: f64,
    pub estimated_intensity_mmi: f64,
    pub enriched_at_ms: i64,
    pub analysis_version: String,
}

// ─── Alert event ──────────────────────────────────────────────────────────────

/// Decoded from the `earthquakes.alerts` Avro topic.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AlertEvent {
    pub source_id: String,
    pub event_time_ms: i64,
    pub latitude: f64,
    pub longitude: f64,
    pub depth_km: Option<f64>,
    pub magnitude: f64,
    pub ml_magnitude: f64,
    pub region_name: Option<String>,
    pub estimated_intensity_mmi: f64,
    pub estimated_felt_radius_km: f64,
    pub is_aftershock: bool,
    pub alert_level: String,
    pub triggered_at_ms: i64,
}

// ─── Server → client message ──────────────────────────────────────────────────

/// The message type sent to WebSocket clients.
///
/// `ServerMessage::Close` is an internal signal to the per-client task to
/// close its connection; it is never serialised.
#[derive(Debug, Clone)]
pub enum ServerMessage {
    Earthquake(Box<EnrichedEvent>),
    Alert(AlertEvent),
    /// Internal-only: instructs the per-client task to send a Close frame.
    Close,
}

impl ServerMessage {
    /// Serialise to a JSON string for transmission over WebSocket.
    ///
    /// Returns `None` for `Close` (internal signal, not transmitted).
    pub fn to_json(&self) -> Option<String> {
        // Use a separate, internally-tagged enum for serialisation so the
        // type tag is injected without adding a field to the domain structs.
        #[derive(Serialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum WireMsg<'a> {
            Earthquake(&'a EnrichedEvent),
            Alert(&'a AlertEvent),
        }

        match self {
            Self::Earthquake(e) => serde_json::to_string(&WireMsg::Earthquake(e)).ok(),
            Self::Alert(a) => serde_json::to_string(&WireMsg::Alert(a)).ok(),
            Self::Close => None,
        }
    }
}
