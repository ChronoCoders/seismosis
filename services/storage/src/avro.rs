use std::collections::HashMap;
use std::io::Cursor;

use apache_avro::{from_avro_datum, types::Value};
use tracing::debug;

use crate::{
    error::ProcessError,
    model::RawFields,
    registry::SchemaRegistryClient,
};

/// Size of the Confluent wire format header: 1 magic byte + 4 schema-ID bytes.
const WIRE_HEADER_LEN: usize = 5;

/// Deserializes messages in Confluent Avro wire format.
///
/// Wire layout: `0x00 | schema_id (4 bytes, big-endian) | avro_datum`
///
/// Schema lookup is delegated to `SchemaRegistryClient`, which caches parsed
/// schemas so the registry is contacted at most once per schema ID per process
/// lifetime.
pub struct AvroDecoder {
    registry: SchemaRegistryClient,
}

impl AvroDecoder {
    pub fn new(registry: SchemaRegistryClient) -> Self {
        Self { registry }
    }

    /// Decode a Confluent-framed payload into `RawFields`.
    pub async fn decode(&self, payload: &[u8]) -> Result<RawFields, ProcessError> {
        // ── Wire header ───────────────────────────────────────────────────────
        if payload.len() < WIRE_HEADER_LEN {
            return Err(ProcessError::InvalidWireHeader(format!(
                "payload is {} bytes, minimum is {}",
                payload.len(),
                WIRE_HEADER_LEN,
            )));
        }
        if payload[0] != 0x00 {
            return Err(ProcessError::InvalidWireHeader(format!(
                "magic byte is 0x{:02x}, expected 0x00",
                payload[0],
            )));
        }

        let schema_id = u32::from_be_bytes([payload[1], payload[2], payload[3], payload[4]]);
        let datum = &payload[WIRE_HEADER_LEN..];

        debug!(schema_id, datum_bytes = datum.len(), "Decoding Avro datum");

        // ── Schema lookup ─────────────────────────────────────────────────────
        let schema = self.registry.get_schema(schema_id).await?;

        // ── Avro decode ───────────────────────────────────────────────────────
        let value = from_avro_datum(&schema, &mut Cursor::new(datum), None)
            .map_err(|e| ProcessError::AvroDecode(e.to_string()))?;

        extract_fields(value)
    }
}

// ─── Field extraction ─────────────────────────────────────────────────────────

fn extract_fields(value: Value) -> Result<RawFields, ProcessError> {
    let fields = match value {
        Value::Record(fields) => fields,
        other => {
            return Err(ProcessError::AvroDecode(format!(
                "expected Record, got {:?}",
                other
            )))
        }
    };

    let mut map: HashMap<String, Value> = fields.into_iter().collect();

    Ok(RawFields {
        source_id: take_string(&mut map, "source_id")?,
        source_network: take_string(&mut map, "source_network")?,
        event_time_ms: take_timestamp_ms(&mut map, "event_time_ms")?,
        latitude: take_double(&mut map, "latitude")?,
        longitude: take_double(&mut map, "longitude")?,
        depth_km: take_optional_double(&mut map, "depth_km")?,
        magnitude: take_double(&mut map, "magnitude")?,
        magnitude_type: take_string(&mut map, "magnitude_type")?,
        region_name: take_optional_string(&mut map, "region_name")?,
        quality_indicator: take_string(&mut map, "quality_indicator")?,
        raw_payload: take_string(&mut map, "raw_payload")?,
        ingested_at_ms: take_timestamp_ms(&mut map, "ingested_at_ms")?,
        pipeline_version: take_string(&mut map, "pipeline_version")?,
    })
}

// ─── Typed field accessors ────────────────────────────────────────────────────

fn take_string(
    map: &mut HashMap<String, Value>,
    field: &'static str,
) -> Result<String, ProcessError> {
    match map.remove(field) {
        Some(Value::String(s)) => Ok(s),
        Some(other) => Err(ProcessError::AvroDecode(format!(
            "field '{}': expected String, got {:?}",
            field, other
        ))),
        None => Err(ProcessError::AvroDecode(format!("field '{}' missing", field))),
    }
}

fn take_double(
    map: &mut HashMap<String, Value>,
    field: &'static str,
) -> Result<f64, ProcessError> {
    match map.remove(field) {
        Some(Value::Double(d)) => Ok(d),
        Some(other) => Err(ProcessError::AvroDecode(format!(
            "field '{}': expected Double, got {:?}",
            field, other
        ))),
        None => Err(ProcessError::AvroDecode(format!("field '{}' missing", field))),
    }
}

fn take_timestamp_ms(
    map: &mut HashMap<String, Value>,
    field: &'static str,
) -> Result<i64, ProcessError> {
    match map.remove(field) {
        // apache-avro 0.16 decodes timestamp-millis as TimestampMillis.
        Some(Value::TimestampMillis(ms)) => Ok(ms),
        // Fallback: raw Long if the schema logical type is not recognised.
        Some(Value::Long(ms)) => Ok(ms),
        Some(other) => Err(ProcessError::AvroDecode(format!(
            "field '{}': expected TimestampMillis or Long, got {:?}",
            field, other
        ))),
        None => Err(ProcessError::AvroDecode(format!("field '{}' missing", field))),
    }
}

fn take_optional_double(
    map: &mut HashMap<String, Value>,
    field: &'static str,
) -> Result<Option<f64>, ProcessError> {
    match map.remove(field) {
        Some(Value::Union(_, inner)) => match *inner {
            Value::Null => Ok(None),
            Value::Double(d) => Ok(Some(d)),
            other => Err(ProcessError::AvroDecode(format!(
                "field '{}': expected Null or Double in union, got {:?}",
                field, other
            ))),
        },
        // Tolerate bare Null / Double outside a Union wrapper.
        Some(Value::Null) => Ok(None),
        Some(Value::Double(d)) => Ok(Some(d)),
        Some(other) => Err(ProcessError::AvroDecode(format!(
            "field '{}': expected optional Double, got {:?}",
            field, other
        ))),
        None => Ok(None),
    }
}

fn take_optional_string(
    map: &mut HashMap<String, Value>,
    field: &'static str,
) -> Result<Option<String>, ProcessError> {
    match map.remove(field) {
        Some(Value::Union(_, inner)) => match *inner {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s)),
            other => Err(ProcessError::AvroDecode(format!(
                "field '{}': expected Null or String in union, got {:?}",
                field, other
            ))),
        },
        Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(other) => Err(ProcessError::AvroDecode(format!(
            "field '{}': expected optional String, got {:?}",
            field, other
        ))),
        None => Ok(None),
    }
}
