/// Schema Registry subject for the raw earthquake topic.
pub const SCHEMA_SUBJECT: &str = "earthquakes.raw-value";

/// Avro schema for `RawEarthquakeEvent`.
///
/// Rules:
/// - All timestamp fields use `timestamp-millis` logical type (UTC epoch ms).
/// - Nullable fields use `["null", T]` union with `"default": null` so that
///   schema evolution can add nullable fields backward-compatibly.
/// - `raw_payload` preserves the original source JSON for audit / replay.
pub const AVRO_SCHEMA: &str = r#"
{
  "type": "record",
  "name": "RawEarthquakeEvent",
  "namespace": "com.seismosis",
  "doc": "Raw seismic event as ingested from an external source. Not yet validated or enriched.",
  "fields": [
    {
      "name": "source_id",
      "type": "string",
      "doc": "Canonical dedup key. Format: {NETWORK_UPPERCASE}:{event_id}. E.g. USGS:us6000m0wb"
    },
    {
      "name": "source_network",
      "type": "string",
      "doc": "Originating network/catalog (USGS, EMSC)"
    },
    {
      "name": "event_time_ms",
      "type": { "type": "long", "logicalType": "timestamp-millis" },
      "doc": "Event origin time as Unix epoch milliseconds (UTC)"
    },
    { "name": "latitude",  "type": "double", "doc": "WGS-84 latitude  [-90, 90]"  },
    { "name": "longitude", "type": "double", "doc": "WGS-84 longitude [-180, 180]" },
    {
      "name": "depth_km",
      "type": ["null", "double"],
      "default": null,
      "doc": "Hypocentral depth in km below surface (positive = deeper)"
    },
    { "name": "magnitude", "type": "double" },
    {
      "name": "magnitude_type",
      "type": "string",
      "doc": "Magnitude scale: ML, Mw, mb, Ms, etc. Always uppercase."
    },
    {
      "name": "region_name",
      "type": ["null", "string"],
      "default": null,
      "doc": "Human-readable region from source (USGS place / EMSC flynn_region)"
    },
    {
      "name": "quality_indicator",
      "type": "string",
      "doc": "A=reviewed, B=estimated, C=preliminary, D=not reviewed"
    },
    {
      "name": "raw_payload",
      "type": "string",
      "doc": "Original source JSON serialised as a string. Preserved for audit and downstream reprocessing."
    },
    {
      "name": "ingested_at_ms",
      "type": { "type": "long", "logicalType": "timestamp-millis" },
      "doc": "Wall-clock time when this service first received this event (UTC)"
    },
    {
      "name": "pipeline_version",
      "type": "string",
      "doc": "Semver of the ingestion service binary that produced this record"
    }
  ]
}
"#;

/// Normalised seismic event ready for Avro encoding.
///
/// All fields map 1:1 to the Avro schema above. The struct is intentionally
/// flat — no nested types — so the Avro encoding path stays simple.
#[derive(Debug, Clone)]
pub struct RawEarthquakeEvent {
    pub source_id: String,
    pub source_network: String,
    /// UTC, Unix epoch milliseconds.
    pub event_time_ms: i64,
    pub latitude: f64,
    pub longitude: f64,
    /// Positive = deeper below surface. `None` when the source omits depth.
    pub depth_km: Option<f64>,
    pub magnitude: f64,
    /// Always stored uppercase (e.g. "ML", "MW", "MB").
    pub magnitude_type: String,
    pub region_name: Option<String>,
    /// Single character: A / B / C / D.
    pub quality_indicator: String,
    /// Original source JSON, preserved verbatim.
    pub raw_payload: String,
    /// UTC, Unix epoch milliseconds.
    pub ingested_at_ms: i64,
    pub pipeline_version: String,
}
