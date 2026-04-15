use std::{collections::HashMap, sync::Arc, time::Duration};

use apache_avro::Schema;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::RwLock;
use tokio_retry::{strategy::ExponentialBackoff, RetryIf};
use tracing::{debug, warn};

use crate::error::{ProcessError, StorageError};

#[derive(Deserialize)]
struct SchemaResponse {
    schema: String,
}

/// Schema Registry client for the storage service.
///
/// Fetches Avro schemas by numeric ID via `GET /schemas/ids/{id}`. The ID is
/// embedded in the 5-byte Confluent wire header of every message produced by
/// the ingestion service. Schemas are immutable once registered, so they are
/// cached indefinitely after the first fetch.
pub struct SchemaRegistryClient {
    base_url: String,
    client: Client,
    /// Schema cache: ID → parsed `Schema`. Never evicted; IDs are permanent.
    cache: Arc<RwLock<HashMap<u32, Arc<Schema>>>>,
}

impl SchemaRegistryClient {
    pub fn new(base_url: String) -> Result<Self, StorageError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| StorageError::SchemaRegistry {
                url: base_url.clone(),
                detail: format!("HTTP client build failed: {}", e),
            })?;

        Ok(Self {
            base_url,
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Return the parsed Avro `Schema` for `schema_id`.
    ///
    /// Checks the in-process cache first (shared read lock). On a miss,
    /// fetches from the registry under a write lock. Concurrent fetches for
    /// the same ID are harmless because the registry is idempotent and the
    /// cache insert is guarded by `entry().or_insert_with`.
    pub async fn get_schema(&self, schema_id: u32) -> Result<Arc<Schema>, ProcessError> {
        // Fast path — read lock.
        {
            let cache = self.cache.read().await;
            if let Some(schema) = cache.get(&schema_id) {
                return Ok(Arc::clone(schema));
            }
        }

        // Slow path — fetch and populate cache.
        let schema = Arc::new(self.fetch_schema(schema_id).await?);

        {
            let mut cache = self.cache.write().await;
            // Another task may have raced us. Insert only if still absent.
            cache
                .entry(schema_id)
                .or_insert_with(|| Arc::clone(&schema));
        }

        Ok(schema)
    }

    async fn fetch_schema(&self, schema_id: u32) -> Result<Schema, ProcessError> {
        let url = format!("{}/schemas/ids/{}", self.base_url, schema_id);

        // Start at 200 ms to keep startup latency low; schema fetches happen
        // on the first message of each new schema ID, which is on the hot path.
        let strategy = ExponentialBackoff::from_millis(200)
            .factor(2)
            .max_delay(Duration::from_secs(30))
            .take(5);

        let schema_json = RetryIf::spawn(
            strategy,
            || async {
                debug!(schema_id, url = %url, "Fetching schema from registry");

                let response = self
                    .client
                    .get(&url)
                    .send()
                    .await
                    .map_err(|e| ProcessError::SchemaRegistry {
                        url: url.clone(),
                        http_status: None,
                        detail: format!("HTTP send failed: {}", e),
                    })?;

                let status = response.status();

                if status.is_success() {
                    let body: SchemaResponse =
                        response.json().await.map_err(|e| ProcessError::SchemaRegistry {
                            url: url.clone(),
                            http_status: Some(status.as_u16()),
                            detail: format!("response parse failed: {}", e),
                        })?;
                    debug!(schema_id, "Schema fetched from registry");
                    Ok(body.schema)
                } else {
                    let detail = format!("HTTP {}: schema_id={}", status.as_u16(), schema_id);
                    warn!(url = %url, status = status.as_u16(), %detail, "Schema Registry returned error");
                    Err(ProcessError::SchemaRegistry {
                        url: url.clone(),
                        http_status: Some(status.as_u16()),
                        detail,
                    })
                }
            },
            |err: &ProcessError| {
                // 4xx = client error (unknown schema ID, misconfigured URL).
                // These are permanent and will not resolve on retry.
                if let ProcessError::SchemaRegistry {
                    http_status: Some(s),
                    ..
                } = err
                {
                    return !(*s >= 400 && *s < 500);
                }
                true // network errors (http_status = None) and 5xx are retryable
            },
        )
        .await?;

        Schema::parse_str(&schema_json)
            .map_err(|e| ProcessError::AvroDecode(format!("schema parse failed: {}", e)))
    }
}
