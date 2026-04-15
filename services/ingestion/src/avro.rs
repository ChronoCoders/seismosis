use apache_avro::{to_avro_datum, types::Value, Schema};

use crate::error::IngestError;
use crate::schema::RawEarthquakeEvent;

/// Encodes `RawEarthquakeEvent` into the Confluent Avro wire format:
///
/// ```text
/// ┌────────┬────────────────────────────┬───── avro datum ─────┐
/// │ 0x00   │  schema_id  (4 B, big-end) │  Avro binary datum   │
/// └────────┴────────────────────────────┴──────────────────────┘
/// ```
///
/// The schema ID is resolved once at service startup by querying the Schema
/// Registry. Consumers decode by reading the schema_id and fetching the
/// writer schema from the registry on first encounter.
pub struct AvroEncoder {
    schema: Schema,
    /// Confluent schema ID prefix bytes (magic byte + 4-byte big-endian u32).
    header: [u8; 5],
}

impl AvroEncoder {
    /// # Panics
    ///
    /// Panics if `schema_id > u32::MAX` — not possible since the Schema
    /// Registry uses 32-bit IDs, but the precondition is explicit.
    pub fn new(schema: Schema, schema_id: u32) -> Self {
        let mut header = [0u8; 5];
        // Byte 0: magic byte (always 0x00 per Confluent wire format spec).
        header[0] = 0x00;
        // Bytes 1-4: schema ID, big-endian.
        header[1..5].copy_from_slice(&schema_id.to_be_bytes());
        Self { schema, header }
    }

    /// Encode an event into the Confluent Avro wire format.
    pub fn encode(&self, event: &RawEarthquakeEvent) -> Result<Vec<u8>, IngestError> {
        let avro_value = event_to_avro(event);

        // to_avro_datum produces raw Avro binary without any container header.
        let datum = to_avro_datum(&self.schema, avro_value).map_err(IngestError::AvroCoding)?;

        let mut buf = Vec::with_capacity(5 + datum.len());
        buf.extend_from_slice(&self.header);
        buf.extend_from_slice(&datum);
        Ok(buf)
    }
}

// ─── Avro Value construction ──────────────────────────────────────────────────

fn event_to_avro(e: &RawEarthquakeEvent) -> Value {
    Value::Record(vec![
        ("source_id".into(), Value::String(e.source_id.clone())),
        (
            "source_network".into(),
            Value::String(e.source_network.clone()),
        ),
        (
            "event_time_ms".into(),
            Value::TimestampMillis(e.event_time_ms),
        ),
        ("latitude".into(), Value::Double(e.latitude)),
        ("longitude".into(), Value::Double(e.longitude)),
        ("depth_km".into(), nullable_double(e.depth_km)),
        ("magnitude".into(), Value::Double(e.magnitude)),
        (
            "magnitude_type".into(),
            Value::String(e.magnitude_type.clone()),
        ),
        (
            "region_name".into(),
            nullable_string(e.region_name.as_deref()),
        ),
        (
            "quality_indicator".into(),
            Value::String(e.quality_indicator.clone()),
        ),
        ("raw_payload".into(), Value::String(e.raw_payload.clone())),
        (
            "ingested_at_ms".into(),
            Value::TimestampMillis(e.ingested_at_ms),
        ),
        (
            "pipeline_version".into(),
            Value::String(e.pipeline_version.clone()),
        ),
    ])
}

/// Construct an Avro `["null", "double"]` union value.
///
/// Union variant index 0 = null (first branch), 1 = double (second branch).
#[inline]
fn nullable_double(v: Option<f64>) -> Value {
    match v {
        Some(d) => Value::Union(1, Box::new(Value::Double(d))),
        None => Value::Union(0, Box::new(Value::Null)),
    }
}

/// Construct an Avro `["null", "string"]` union value.
#[inline]
fn nullable_string(v: Option<&str>) -> Value {
    match v {
        Some(s) => Value::Union(1, Box::new(Value::String(s.to_owned()))),
        None => Value::Union(0, Box::new(Value::Null)),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use apache_avro::from_avro_datum;
    use std::collections::HashMap;

    use crate::schema::{RawEarthquakeEvent, AVRO_SCHEMA};

    fn test_event() -> RawEarthquakeEvent {
        RawEarthquakeEvent {
            source_id: "USGS:us6000test".to_owned(),
            source_network: "USGS".to_owned(),
            event_time_ms: 1_700_000_000_000,
            latitude: 37.5,
            longitude: -122.1,
            depth_km: Some(10.0),
            magnitude: 4.5,
            magnitude_type: "ML".to_owned(),
            region_name: Some("San Francisco Bay Area".to_owned()),
            quality_indicator: "A".to_owned(),
            raw_payload: r#"{"id":"us6000test"}"#.to_owned(),
            ingested_at_ms: 1_700_000_001_000,
            pipeline_version: "0.1.0".to_owned(),
        }
    }

    fn decode_record(encoded: &[u8], schema: &Schema) -> HashMap<String, Value> {
        // Skip the 5-byte Confluent wire header before decoding.
        let mut datum = &encoded[5..];
        let decoded = from_avro_datum(schema, &mut datum, None).expect("Avro decoding failed");
        if let Value::Record(fields) = decoded {
            fields.into_iter().collect()
        } else {
            panic!("Expected Avro Record, got {:?}", decoded);
        }
    }

    // ── Confluent wire header ─────────────────────────────────────────────

    #[test]
    fn wire_header_magic_byte_is_zero() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema, 42);
        let bytes = encoder.encode(&test_event()).unwrap();
        assert_eq!(bytes[0], 0x00, "Magic byte must be 0x00");
    }

    #[test]
    fn wire_header_schema_id_big_endian() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema, 42);
        let bytes = encoder.encode(&test_event()).unwrap();
        assert_eq!(
            &bytes[1..5],
            &42u32.to_be_bytes(),
            "Schema ID must be 4-byte big-endian"
        );
    }

    #[test]
    fn wire_header_schema_id_max_value() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema, u32::MAX);
        let bytes = encoder.encode(&test_event()).unwrap();
        assert_eq!(&bytes[1..5], &u32::MAX.to_be_bytes());
    }

    #[test]
    fn wire_header_followed_by_avro_datum() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema, 1);
        let bytes = encoder.encode(&test_event()).unwrap();
        assert!(
            bytes.len() > 5,
            "Encoded output must contain data beyond the 5-byte header"
        );
    }

    // ── event_to_avro round-trip ──────────────────────────────────────────

    #[test]
    fn roundtrip_required_string_fields() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema.clone(), 1);
        let event = test_event();
        let encoded = encoder.encode(&event).unwrap();
        let fields = decode_record(&encoded, &schema);

        assert_eq!(fields["source_id"], Value::String("USGS:us6000test".into()));
        assert_eq!(fields["source_network"], Value::String("USGS".into()));
        assert_eq!(fields["magnitude_type"], Value::String("ML".into()));
        assert_eq!(fields["quality_indicator"], Value::String("A".into()));
        assert_eq!(fields["pipeline_version"], Value::String("0.1.0".into()));
    }

    #[test]
    fn roundtrip_numeric_fields() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema.clone(), 1);
        let event = test_event();
        let encoded = encoder.encode(&event).unwrap();
        let fields = decode_record(&encoded, &schema);

        assert_eq!(fields["latitude"], Value::Double(37.5));
        assert_eq!(fields["longitude"], Value::Double(-122.1));
        assert_eq!(fields["magnitude"], Value::Double(4.5));
        assert_eq!(
            fields["event_time_ms"],
            Value::TimestampMillis(1_700_000_000_000)
        );
        assert_eq!(
            fields["ingested_at_ms"],
            Value::TimestampMillis(1_700_000_001_000)
        );
    }

    #[test]
    fn roundtrip_nullable_depth_some() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema.clone(), 1);
        let encoded = encoder.encode(&test_event()).unwrap();
        let fields = decode_record(&encoded, &schema);

        assert_eq!(
            fields["depth_km"],
            Value::Union(1, Box::new(Value::Double(10.0))),
            "depth_km Some(10.0) must decode as Union(1, Double(10.0))"
        );
    }

    #[test]
    fn roundtrip_nullable_depth_none() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema.clone(), 1);
        let mut event = test_event();
        event.depth_km = None;
        let encoded = encoder.encode(&event).unwrap();
        let fields = decode_record(&encoded, &schema);

        assert_eq!(
            fields["depth_km"],
            Value::Union(0, Box::new(Value::Null)),
            "depth_km None must decode as Union(0, Null)"
        );
    }

    #[test]
    fn roundtrip_nullable_region_name_some() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema.clone(), 1);
        let encoded = encoder.encode(&test_event()).unwrap();
        let fields = decode_record(&encoded, &schema);

        assert_eq!(
            fields["region_name"],
            Value::Union(1, Box::new(Value::String("San Francisco Bay Area".into())))
        );
    }

    #[test]
    fn roundtrip_nullable_region_name_none() {
        let schema = Schema::parse_str(AVRO_SCHEMA).unwrap();
        let encoder = AvroEncoder::new(schema.clone(), 1);
        let mut event = test_event();
        event.region_name = None;
        let encoded = encoder.encode(&event).unwrap();
        let fields = decode_record(&encoded, &schema);

        assert_eq!(
            fields["region_name"],
            Value::Union(0, Box::new(Value::Null))
        );
    }
}
