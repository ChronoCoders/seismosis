use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio_retry::{strategy::ExponentialBackoff, RetryIf};
use tracing::{debug, warn};

use crate::error::IngestError;

/// Minimal Confluent Schema Registry client.
///
/// Covers the single operation needed at startup: register (or retrieve the
/// existing ID for) a schema. The Schema Registry is idempotent — re-posting
/// the identical schema JSON returns the same schema ID.
pub struct SchemaRegistryClient {
    base_url: String,
    client: Client,
}

#[derive(Serialize)]
struct RegisterRequest<'a> {
    schema: &'a str,
}

#[derive(Deserialize)]
struct RegisterResponse {
    id: u32,
}

/// Error body returned by the Schema Registry on 4xx/5xx.
#[derive(Deserialize, Default)]
struct SrErrorBody {
    #[serde(default)]
    error_code: i32,
    #[serde(default)]
    message: String,
}

impl SchemaRegistryClient {
    pub fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    /// Register `schema_json` under `subject`.
    ///
    /// If the schema is already registered the registry returns the existing
    /// ID — no duplicate is created. Retries up to 5 times with exponential
    /// backoff starting at 2 s (handles the case where Redpanda starts slowly).
    pub async fn register_schema(
        &self,
        subject: &str,
        schema_json: &str,
    ) -> Result<u32, IngestError> {
        let url = format!("{}/subjects/{}/versions", self.base_url, subject);
        let body = RegisterRequest {
            schema: schema_json,
        };

        let strategy = ExponentialBackoff::from_millis(2_000)
            .factor(2)
            .max_delay(Duration::from_secs(30))
            .take(5);

        // Use RetryIf so that 4xx responses (bad schema JSON, incompatible
        // schema, wrong subject name) abort immediately — they are configuration
        // errors that will not resolve on retry. 5xx responses and network
        // errors are treated as transient and retried with backoff.
        //
        // The detail string is formatted as "HTTP {status}: ..." for all HTTP
        // error responses, so "HTTP 4" reliably identifies the 4xx class.
        RetryIf::spawn(
            strategy,
            || async {
                debug!(url = %url, subject, "Registering Avro schema");

                let response = self
                    .client
                    .post(&url)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        "application/vnd.schemaregistry.v1+json",
                    )
                    .json(&body)
                    .send()
                    .await
                    .map_err(|e| IngestError::SchemaRegistry {
                        url: url.clone(),
                        detail: format!("HTTP send failed: {}", e),
                    })?;

                let status = response.status();

                if status.is_success() {
                    let reg: RegisterResponse =
                        response
                            .json()
                            .await
                            .map_err(|e| IngestError::SchemaRegistry {
                                url: url.clone(),
                                detail: format!("response parse failed: {}", e),
                            })?;

                    debug!(schema_id = reg.id, subject, "Schema registered/found");
                    Ok(reg.id)
                } else {
                    // Read the raw body first so we can log it if JSON parsing
                    // fails (e.g. an nginx 502 HTML page instead of SR JSON).
                    let body_bytes = response.bytes().await.unwrap_or_default();
                    let err: SrErrorBody =
                        serde_json::from_slice(&body_bytes).unwrap_or_else(|_| {
                            warn!(
                                url = %url,
                                raw_body = %String::from_utf8_lossy(&body_bytes),
                                "Schema Registry error body was not valid JSON",
                            );
                            SrErrorBody::default()
                        });
                    let detail = format!(
                        "HTTP {}: error_code={} message={}",
                        status, err.error_code, err.message
                    );
                    warn!(url = %url, %detail, "Schema Registry returned error");
                    Err(IngestError::SchemaRegistry {
                        url: url.clone(),
                        detail,
                    })
                }
            },
            |err: &IngestError| {
                if let IngestError::SchemaRegistry { detail, .. } = err {
                    // "HTTP 4xx: ..." — client error, not retryable.
                    return !detail.starts_with("HTTP 4");
                }
                true // network errors and 5xx are retryable
            },
        )
        .await
    }
}
