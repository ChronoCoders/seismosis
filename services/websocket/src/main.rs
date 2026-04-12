//! Seismosis WebSocket Push Service
//!
//! Consumes enriched earthquake events and alerts from Redpanda, evaluates
//! per-client subscription filters, and delivers matching events to connected
//! browser clients in real time over WebSocket.
//!
//! ## Topics consumed
//!
//! | Topic                    | Message type     | Avro subject                  |
//! |--------------------------|------------------|-------------------------------|
//! | `earthquakes.enriched`   | `EnrichedEvent`  | `earthquakes.enriched-value`  |
//! | `earthquakes.alerts`     | `AlertEvent`     | `earthquakes.alerts-value`    |
//!
//! ## Lifecycle
//!
//! 1. Load config from environment.
//! 2. Pre-warm the Schema Registry cache for both known subjects.
//! 3. Create the Kafka `StreamConsumer` (subscribed to both topics).
//! 4. Create the client `Hub` (shared registry + fan-out).
//! 5. Start the Prometheus metrics HTTP server.
//! 6. Spawn the Kafka consume loop.
//! 7. Enter the WebSocket accept loop (runs on the main task).
//! 8. On SIGTERM / SIGINT: signal all tasks, close all clients, drain.

mod avro;
mod config;
mod error;
mod event;
mod filter;
mod handler;
mod hub;
mod metrics;

use std::sync::Arc;
use std::time::Instant;

use apache_avro::from_value;
use axum::{body::Body, http::Response, routing::get, Router};
use futures::StreamExt;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::Message;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tracing::{error, info, warn};

use avro::{AvroDecoder, SchemaCache};
use config::Config;
use event::{AlertEvent, EnrichedEvent};
use hub::Hub;
use metrics::Metrics;

// Schema Registry subjects matching the analysis service's schema.py
const ENRICHED_SUBJECT: &str = "earthquakes.enriched-value";
const ALERTS_SUBJECT: &str = "earthquakes.alerts-value";

// ─── Entry point ─────────────────────────────────────────────────────────────

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // ── Tracing / structured logging ──────────────────────────────────────
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(true)
        .with_thread_ids(false)
        .with_current_span(false)
        .init();

    let config = Arc::new(Config::from_env()?);

    info!(
        ws_port = config.ws_port,
        metrics_port = config.metrics_port,
        kafka_brokers = config.kafka_brokers.as_str(),
        topics = format!("{}, {}", config.kafka_topic_enriched, config.kafka_topic_alerts).as_str(),
        group_id = config.kafka_group_id.as_str(),
        "Seismosis WebSocket service starting",
    );

    // ── HTTP client for Schema Registry ──────────────────────────────────
    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent(concat!("seismosis-websocket/", env!("CARGO_PKG_VERSION")))
        .build()?;

    // ── Schema Registry cache + Avro decoder ─────────────────────────────
    let schema_cache = SchemaCache::new(config.schema_registry_url.clone(), http_client.clone());
    let decoder = Arc::new(AvroDecoder::new(Arc::clone(&schema_cache)));

    // Pre-warm the cache by resolving both subjects to schema IDs.
    // This ensures the first consumed message doesn't pay a registry RTT and
    // surfaces misconfiguration (wrong subject names, SR unreachable) at startup.
    pre_warm_cache(&schema_cache, &config).await?;

    // ── Kafka consumer ────────────────────────────────────────────────────
    // auto.offset.reset = latest: the WebSocket service is real-time.
    // Historical replay is the REST API's responsibility, not ours.
    // enable.auto.commit = false: we call store_offset_from_message() after
    // each successful fan-out so offsets advance even if 0 clients match.
    let consumer: Arc<StreamConsumer> = Arc::new(
        ClientConfig::new()
            .set("bootstrap.servers", &config.kafka_brokers)
            .set("group.id", &config.kafka_group_id)
            .set("auto.offset.reset", "latest")
            .set("enable.auto.commit", "true")
            .set("auto.commit.interval.ms", "5000")
            .set("session.timeout.ms", "30000")
            .set("heartbeat.interval.ms", "10000")
            .set("log.connection.close", "false")
            .create()
            .map_err(|e| anyhow::anyhow!("Kafka consumer creation failed: {e}"))?,
    );

    consumer
        .subscribe(&[
            config.kafka_topic_enriched.as_str(),
            config.kafka_topic_alerts.as_str(),
        ])
        .map_err(|e| anyhow::anyhow!("Kafka subscribe failed: {e}"))?;

    info!(
        topic_enriched = config.kafka_topic_enriched.as_str(),
        topic_alerts = config.kafka_topic_alerts.as_str(),
        "Kafka consumer subscribed",
    );

    // ── Prometheus metrics ────────────────────────────────────────────────
    let metrics = Metrics::new()?;

    // ── Client hub ────────────────────────────────────────────────────────
    let hub = Hub::new(Arc::clone(&metrics), config.client_channel_capacity);

    // ── Shutdown channel ──────────────────────────────────────────────────
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // ── Metrics HTTP server ───────────────────────────────────────────────
    let metrics_handle = {
        let m = Arc::clone(&metrics);
        let port = config.metrics_port;
        let rx = shutdown_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(m, port, rx).await {
                error!(error = %e, "Metrics server failed");
            }
        })
    };

    // ── Kafka consume loop ────────────────────────────────────────────────
    let consume_handle = tokio::spawn(run_consume_loop(
        Arc::clone(&consumer),
        Arc::clone(&decoder),
        Arc::clone(&hub),
        Arc::clone(&metrics),
        Arc::clone(&config),
        shutdown_rx.clone(),
    ));

    // ── WebSocket accept loop ─────────────────────────────────────────────
    let ws_addr = format!("0.0.0.0:{}", config.ws_port);
    let ws_listener = TcpListener::bind(&ws_addr)
        .await
        .map_err(|e| anyhow::anyhow!("WebSocket listener bind failed on {ws_addr}: {e}"))?;

    info!(port = config.ws_port, "WebSocket server listening");

    let accept_handle = tokio::spawn(run_accept_loop(
        ws_listener,
        Arc::clone(&hub),
        Arc::clone(&metrics),
        Arc::clone(&config),
        shutdown_rx.clone(),
    ));

    // ── Await shutdown signal ─────────────────────────────────────────────
    shutdown_signal().await;
    info!("Shutdown signal received — beginning graceful drain");

    let _ = shutdown_tx.send(true);

    // Ask the hub to send Close frames to all connected clients.
    hub.close_all().await;

    // Wait for consumer and accept loop to finish.
    if let Err(e) = consume_handle.await {
        error!(error = ?e, "Consume loop task panicked");
    }
    if let Err(e) = accept_handle.await {
        error!(error = ?e, "Accept loop task panicked");
    }
    if let Err(e) = metrics_handle.await {
        error!(error = ?e, "Metrics server task panicked");
    }

    info!("Seismosis WebSocket service stopped cleanly");
    Ok(())
}

// ─── Schema Registry pre-warm ─────────────────────────────────────────────────

async fn pre_warm_cache(
    cache: &SchemaCache,
    config: &Config,
) -> anyhow::Result<()> {
    // Resolve subject → schema_id → parse and cache both schemas so the first
    // real message is handled without a synchronous registry round-trip.
    for (subject, path) in [
        (ENRICHED_SUBJECT, "versions/latest"),
        (ALERTS_SUBJECT, "versions/latest"),
    ] {
        let url = format!(
            "{}/subjects/{}/{}",
            config.schema_registry_url, subject, path
        );
        let resp = reqwest::get(&url)
            .await
            .map_err(|e| anyhow::anyhow!("Schema Registry pre-warm GET {url}: {e}"))?;

        if !resp.status().is_success() {
            return Err(anyhow::anyhow!(
                "Schema Registry pre-warm: subject '{}' returned HTTP {}",
                subject,
                resp.status()
            ));
        }

        #[derive(serde::Deserialize)]
        struct SubjectVersion {
            id: u32,
        }
        let sv: SubjectVersion = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Schema Registry pre-warm response parse: {e}"))?;

        // Fetch by ID to populate the decoder's cache.
        cache
            .get(sv.id)
            .await
            .map_err(|e| anyhow::anyhow!("Schema Registry pre-warm schema fetch: {e}"))?;

        info!(subject, schema_id = sv.id, "Schema pre-warmed");
    }
    Ok(())
}

// ─── Kafka consume loop ───────────────────────────────────────────────────────

async fn run_consume_loop(
    consumer: Arc<StreamConsumer>,
    decoder: Arc<AvroDecoder>,
    hub: Arc<Hub>,
    metrics: Arc<Metrics>,
    config: Arc<Config>,
    mut shutdown: watch::Receiver<bool>,
) {
    info!("Consume loop started");
    let mut stream = consumer.stream();

    loop {
        tokio::select! {
            biased;

            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!("Consume loop received shutdown signal — stopping");
                    break;
                }
            }

            msg = stream.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        warn!(error = %e, "Kafka consumer error — continuing");
                        continue;
                    }
                    None => {
                        // Stream ended (consumer closed).
                        break;
                    }
                };

                let topic = msg.topic();
                let payload = match msg.payload() {
                    Some(p) => p,
                    None => {
                        warn!(topic, "Received tombstone message (null payload) — skipping");
                        continue;
                    }
                };

                let t0 = Instant::now();

                match decoder.decode(payload).await {
                    Ok((_schema_id, avro_value)) => {
                        if topic == config.kafka_topic_enriched {
                            match from_value::<EnrichedEvent>(&avro_value) {
                                Ok(event) => {
                                    hub.broadcast_enriched(event).await;
                                    metrics
                                        .broadcast_duration_seconds
                                        .with_label_values(&["enriched"])
                                        .observe(t0.elapsed().as_secs_f64());
                                }
                                Err(e) => {
                                    warn!(
                                        error = %e,
                                        topic,
                                        partition = msg.partition(),
                                        offset = msg.offset(),
                                        "Failed to deserialise EnrichedEvent — skipping"
                                    );
                                }
                            }
                        } else if topic == config.kafka_topic_alerts {
                            match from_value::<AlertEvent>(&avro_value) {
                                Ok(event) => {
                                    hub.broadcast_alert(event).await;
                                    metrics
                                        .broadcast_duration_seconds
                                        .with_label_values(&["alert"])
                                        .observe(t0.elapsed().as_secs_f64());
                                }
                                Err(e) => {
                                    warn!(
                                        error = %e,
                                        topic,
                                        partition = msg.partition(),
                                        offset = msg.offset(),
                                        "Failed to deserialise AlertEvent — skipping"
                                    );
                                }
                            }
                        } else {
                            warn!(topic, "Received message from unexpected topic — skipping");
                        }
                    }
                    Err(e) => {
                        warn!(
                            error = %e,
                            topic,
                            partition = msg.partition(),
                            offset = msg.offset(),
                            "Avro decode failed — skipping message"
                        );
                    }
                }
            }
        }
    }

    // Flush and close the consumer so any pending offset commits are sent.
    drop(stream);
    info!("Consume loop stopped");
}

// ─── WebSocket accept loop ────────────────────────────────────────────────────

async fn run_accept_loop(
    listener: TcpListener,
    hub: Arc<Hub>,
    metrics: Arc<Metrics>,
    config: Arc<Config>,
    mut shutdown: watch::Receiver<bool>,
) {
    info!(port = config.ws_port, "WebSocket accept loop started");

    loop {
        tokio::select! {
            biased;

            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    info!("Accept loop received shutdown — no longer accepting new connections");
                    break;
                }
            }

            result = listener.accept() => {
                match result {
                    Ok((stream, addr)) => {
                        // Spawn an independent task per client so one slow
                        // client cannot block the accept loop or other clients.
                        let hub = Arc::clone(&hub);
                        let metrics = Arc::clone(&metrics);
                        let shutdown_rx = shutdown.clone();
                        let default_mag = config.default_min_magnitude;
                        tokio::spawn(async move {
                            handler::handle_connection(
                                stream,
                                addr,
                                hub,
                                metrics,
                                shutdown_rx,
                                default_mag,
                            )
                            .await;
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "TCP accept failed");
                    }
                }
            }
        }
    }

    info!("WebSocket accept loop stopped");
}

// ─── Metrics HTTP server ──────────────────────────────────────────────────────

async fn run_metrics_server(
    metrics: Arc<Metrics>,
    port: u16,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), error::WsError> {
    use prometheus::Encoder;

    let metrics_ref = Arc::clone(&metrics);

    let app = Router::new()
        .route(
            "/metrics",
            get(move || {
                let m = Arc::clone(&metrics_ref);
                async move {
                    let encoder = prometheus::TextEncoder::new();
                    let mut buf = Vec::new();
                    match encoder.encode(&m.registry.gather(), &mut buf) {
                        Ok(()) => Response::builder()
                            .header(axum::http::header::CONTENT_TYPE, encoder.format_type())
                            .body(Body::from(buf))
                            .unwrap_or_else(|_| {
                                Response::builder()
                                    .status(500)
                                    .body(Body::empty())
                                    .expect("static 500 response")
                            }),
                        Err(e) => {
                            error!(error = %e, "Failed to encode Prometheus metrics");
                            Response::builder()
                                .status(500)
                                .body(Body::from(e.to_string()))
                                .expect("static error response")
                        }
                    }
                }
            }),
        )
        .route("/health", get(|| async { "ok" }));

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| error::WsError::HttpServer(e.to_string()))?;

    info!(port, "Metrics server listening on /metrics and /health");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|v| *v).await;
        })
        .await
        .map_err(|e| error::WsError::HttpServer(e.to_string()))
}

// ─── Shutdown signal ──────────────────────────────────────────────────────────

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    {
        let terminate = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler")
                .recv()
                .await;
        };
        tokio::select! {
            () = ctrl_c => {}
            () = terminate => {}
        }
    }

    #[cfg(not(unix))]
    ctrl_c.await;
}
