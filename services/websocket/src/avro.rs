//! Confluent-format Avro decoder with Schema Registry schema caching.
//!
//! Wire format (Confluent):
//! ```text
//!  ┌────────┬──────────────────────┬──── avro datum ────┐
//!  │ 0x00   │  schema_id (4B, BE)  │  schemaless binary │
//!  └────────┴──────────────────────┴────────────────────┘
//! ```
//!
//! The `SchemaCache` fetches schemas from the Schema Registry on first
//! encounter per schema ID and stores them for the lifetime of the service.
//! Schema IDs are stable — the registry never reassigns an ID to a different
//! schema — so this cache is correct without expiry.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::Arc;

use apache_avro::{Schema, types::Value};
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

use crate::error::WsError;

const HEADER_LEN: usize = 5;
const MAGIC_BYTE: u8 = 0x00;

// ─── Schema Registry cache ────────────────────────────────────────────────────

/// Thread-safe, lazily-populated cache of parsed Avro schemas by schema ID.
pub struct SchemaCache {
    base_url: String,
    client: Client,
    cache: Mutex<HashMap<u32, Schema>>,
}

#[derive(Deserialize)]
struct SchemaResponse {
    schema: String,
}

impl SchemaCache {
    pub fn new(base_url: String, client: Client) -> Arc<Self> {
        Arc::new(Self {
            base_url,
            client,
            cache: Mutex::new(HashMap::new()),
        })
    }

    /// Return the cached `Schema` for `schema_id`.
    ///
    /// On a cache miss the schema is fetched from the registry, parsed, and
    /// inserted into the cache before returning.
    pub async fn get(&self, schema_id: u32) -> Result<Schema, WsError> {
        // Fast path: schema is already cached.
        {
            let guard = self.cache.lock().unwrap();
            if let Some(schema) = guard.get(&schema_id) {
                return Ok(schema.clone());
            }
        }

        // Slow path: fetch from registry.
        let url = format!("{}/schemas/ids/{}", self.base_url, schema_id);
        debug!(%url, schema_id, "Cache miss — fetching schema from registry");

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| WsError::SchemaRegistry {
                url: url.clone(),
                detail: format!("HTTP request failed: {e}"),
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(WsError::SchemaRegistry {
                url,
                detail: format!("HTTP {status}: {body}"),
            });
        }

        let body: SchemaResponse = resp.json().await.map_err(|e| WsError::SchemaRegistry {
            url: url.clone(),
            detail: format!("response parse error: {e}"),
        })?;

        let schema = Schema::parse_str(&body.schema)?;

        {
            let mut guard = self.cache.lock().unwrap();
            guard.insert(schema_id, schema.clone());
        }

        debug!(schema_id, "Schema cached");
        Ok(schema)
    }
}

// ─── Avro decoder ─────────────────────────────────────────────────────────────

/// Decodes Confluent wire-format Avro messages using a shared schema cache.
pub struct AvroDecoder {
    cache: Arc<SchemaCache>,
}

impl AvroDecoder {
    pub fn new(cache: Arc<SchemaCache>) -> Self {
        Self { cache }
    }

    /// Decode `data` from Confluent wire format to an Avro `Value`.
    ///
    /// Returns `(schema_id, Value)`.  Callers use `schema_id` to determine
    /// message type when consuming from a multi-topic subscription.
    pub async fn decode(&self, data: &[u8]) -> Result<(u32, Value), WsError> {
        if data.len() < HEADER_LEN {
            return Err(WsError::InvalidWireFormat(format!(
                "message too short: {} bytes (need at least {HEADER_LEN})",
                data.len()
            )));
        }
        if data[0] != MAGIC_BYTE {
            return Err(WsError::InvalidWireFormat(format!(
                "expected magic byte 0x00, got 0x{:02x}",
                data[0]
            )));
        }

        let schema_id = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        let schema = self.cache.get(schema_id).await?;

        let mut reader = &data[HEADER_LEN..];
        let value = apache_avro::from_avro_datum(&schema, &mut reader, None)?;

        Ok((schema_id, value))
    }
}
