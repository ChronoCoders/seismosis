//! Seismosis Ingestion Service
//!
//! Polls the USGS, EMSC, AFAD, and optional FDSN network APIs (GFZ, INGV),
//! normalises events to a common schema, serialises with Avro against the
//! Redpanda Schema Registry, and publishes to the `earthquakes.raw` topic.
//!
//! ## Lifecycle
//!
//! 1. Load config from environment.
//! 2. Register the Avro schema with the Schema Registry.
//! 3. Connect to Redis (dedup store).
//! 4. Spawn the Prometheus metrics HTTP server.
//! 5. Enter the poll loop; exit cleanly on SIGTERM / SIGINT.

mod avro;
mod config;
mod dead_letter;
mod dedup;
mod error;
mod metrics;
mod producer;
mod registry;
mod schema;
mod sources;

#[cfg(test)]
mod integration_tests;

use std::sync::Arc;
use std::time::{Duration, Instant};

use apache_avro::Schema;
use axum::{body::Body, http::Response, routing::get, Router};
use chrono::Utc;
use futures::future::join_all;
use tokio::sync::watch;
use tracing::{error, info, warn};

use avro::AvroEncoder;
use config::Config;
use dead_letter::{DlqProducer, IngestDlqEnvelope};
use dedup::Deduplicator;
use metrics::Metrics;
use producer::EventProducer;
use registry::SchemaRegistryClient;
use sources::{
    afad::AfadSource,
    emsc::EmscSource,
    fdsn::{FdsnSource, FdsnSourceConfig},
    usgs::UsgsSource,
    SeismicSource,
};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|e| {
                eprintln!("WARN: invalid RUST_LOG filter ({e}), defaulting to info");
                tracing_subscriber::EnvFilter::new("info")
            }),
        )
        .with_target(true)
        .with_thread_ids(false)
        .with_current_span(false)
        .init();

    let config = Arc::new(Config::from_env()?);

    info!(
        version = config.pipeline_version.as_str(),
        kafka_brokers = config.kafka_brokers.as_str(),
        topic = config.kafka_topic_raw.as_str(),
        schema_registry = config.schema_registry_url.as_str(),
        poll_interval_secs = config.poll_interval.as_secs(),
        min_magnitude = config.min_magnitude,
        "Seismosis ingestion service starting",
    );

    let metrics = Arc::new(Metrics::new()?);

    let http_client = reqwest::Client::builder()
        .timeout(config.http_timeout)
        .user_agent(concat!("seismosis-ingestion/", env!("CARGO_PKG_VERSION")))
        .gzip(true)
        .build()?;

    let sr_client =
        SchemaRegistryClient::new(config.schema_registry_url.clone(), http_client.clone());

    let schema_id = sr_client
        .register_schema(schema::SCHEMA_SUBJECT, schema::AVRO_SCHEMA)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to register Avro schema — cannot start");
            e
        })?;

    metrics
        .schema_registry_requests_total
        .with_label_values(&["ok"])
        .inc();

    info!(
        schema_id,
        subject = schema::SCHEMA_SUBJECT,
        "Avro schema registered"
    );

    let avro_schema = Schema::parse_str(schema::AVRO_SCHEMA)?;
    let encoder = Arc::new(AvroEncoder::new(avro_schema, schema_id));

    let deduplicator = Arc::new(Deduplicator::new(&config.redis_url, config.dedup_ttl_secs).await);

    if !deduplicator.is_redis_healthy() {
        warn!(
            "Redis dedup store is unavailable. Running with in-memory LRU fallback. \
             Duplicate events may appear in the topic if the service restarts."
        );
    }

    let kafka_producer = Arc::new(EventProducer::new(&config, Arc::clone(&metrics))?);

    let dlq_producer = Arc::new(
        DlqProducer::new(
            &config.kafka_brokers,
            config.kafka_topic_dead_letter.clone(),
        )
        .map_err(|e| anyhow::anyhow!("dlq producer: {}", e))?,
    );
    info!(
        topic = config.kafka_topic_dead_letter.as_str(),
        "Dead-letter producer ready"
    );

    // Always-on sources.
    let mut sources: Vec<Box<dyn SeismicSource>> = vec![
        Box::new(UsgsSource::new(
            http_client.clone(),
            Arc::clone(&config),
            Arc::clone(&metrics),
        )),
        Box::new(EmscSource::new(
            http_client.clone(),
            Arc::clone(&config),
            Arc::clone(&metrics),
        )),
        Box::new(AfadSource::new(
            http_client.clone(),
            Arc::clone(&config),
            Arc::clone(&metrics),
        )),
    ];

    // Optional FDSN network sources.
    //
    // Priority: if `FDSN_SOURCES` is set and non-empty, parse it as a JSON array of
    // objects `[{"name":"GFZ","prefix":"gfz","base_url":"https://..."}]` and register
    // one FdsnSource per entry. This supersedes the per-network flags below.
    //
    // Fallback (backward-compatible): use the individual `FDSN_*_ENABLED` flags.
    let fdsn_sources_env = std::env::var("FDSN_SOURCES").unwrap_or_default();
    if !fdsn_sources_env.trim().is_empty() {
        #[derive(serde::Deserialize)]
        struct FdsnEntry {
            name: String,
            prefix: String,
            base_url: String,
        }
        match serde_json::from_str::<Vec<FdsnEntry>>(&fdsn_sources_env) {
            Ok(entries) => {
                for entry in entries {
                    let label = entry.name.clone();
                    let cfg = FdsnSourceConfig {
                        name: entry.name,
                        prefix: entry.prefix,
                        base_url: entry.base_url,
                    };
                    info!(source = %label, "FDSN source enabled via FDSN_SOURCES");
                    sources.push(Box::new(FdsnSource::from_config(
                        cfg,
                        http_client.clone(),
                        Arc::clone(&config),
                        Arc::clone(&metrics),
                    )));
                }
            }
            Err(e) => {
                anyhow::bail!(
                    "FDSN_SOURCES is set but could not be parsed as a JSON array: {e}"
                );
            }
        }
    } else {
        // Fallback: individual enable flags (backward-compatible).
        if std::env::var("FDSN_GFZ_ENABLED").as_deref() == Ok("true") {
            info!("FDSN GFZ source enabled");
            sources.push(Box::new(FdsnSource::new(
                "GFZ",
                "gfz",
                std::env::var("FDSN_GFZ_BASE_URL")
                    .unwrap_or_else(|_| "https://geofon.gfz-potsdam.de".into()),
                http_client.clone(),
                Arc::clone(&config),
                Arc::clone(&metrics),
            )));
        }

        if std::env::var("FDSN_INGV_ENABLED").as_deref() == Ok("true") {
            info!("FDSN INGV source enabled");
            sources.push(Box::new(FdsnSource::new(
                "INGV",
                "ingv",
                std::env::var("FDSN_INGV_BASE_URL")
                    .unwrap_or_else(|_| "https://webservices.ingv.it".into()),
                http_client.clone(),
                Arc::clone(&config),
                Arc::clone(&metrics),
            )));
        }

        if std::env::var("FDSN_NIED_ENABLED").as_deref() == Ok("true") {
            info!("FDSN NIED source enabled");
            sources.push(Box::new(FdsnSource::new(
                "NIED",
                "nied",
                std::env::var("FDSN_NIED_BASE_URL")
                    .unwrap_or_else(|_| "https://www.fnet.bosai.go.jp".into()),
                http_client.clone(),
                Arc::clone(&config),
                Arc::clone(&metrics),
            )));
        }
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let metrics_server_handle = {
        let metrics = Arc::clone(&metrics);
        let port = config.metrics_port;
        let shutdown_rx_metrics = shutdown_rx.clone();
        tokio::spawn(async move {
            if let Err(e) = run_metrics_server(metrics, port, shutdown_rx_metrics).await {
                error!(error = %e, "Metrics server failed");
            }
        })
    };

    let poll_handle = tokio::spawn(run_poll_loop(
        sources,
        PollLoopCtx {
            producer: kafka_producer,
            dlq: dlq_producer,
            deduplicator,
            encoder,
            metrics: Arc::clone(&metrics),
            config: Arc::clone(&config),
        },
        shutdown_rx,
    ));

    shutdown_signal().await;
    info!("Shutdown signal received — draining in-flight work");

    // Signal the poll loop and metrics server to stop.
    // send() only fails when all Receivers have been dropped. Both spawned
    // tasks still hold their receivers at this point, so this cannot fail.
    let _ = shutdown_tx.send(true);

    // Wait for the poll loop to finish its current batch before exiting.
    if let Err(e) = poll_handle.await {
        error!(error = ?e, "Poll loop task terminated unexpectedly");
    }
    // Wait for the metrics server to finish draining in-flight scrapes.
    if let Err(e) = metrics_server_handle.await {
        error!(error = ?e, "Metrics server task terminated unexpectedly");
    }
    info!("Seismosis ingestion service stopped cleanly");

    Ok(())
}

/// Shared services threaded through the poll loop.
struct PollLoopCtx {
    producer: Arc<EventProducer>,
    dlq: Arc<DlqProducer>,
    deduplicator: Arc<Deduplicator>,
    encoder: Arc<AvroEncoder>,
    metrics: Arc<Metrics>,
    config: Arc<Config>,
}

async fn run_poll_loop(
    sources: Vec<Box<dyn SeismicSource>>,
    ctx: PollLoopCtx,
    mut shutdown: watch::Receiver<bool>,
) {
    let PollLoopCtx {
        producer,
        dlq,
        deduplicator,
        encoder,
        metrics,
        config,
    } = ctx;
    info!("Poll loop started");

    // Tracks the deadline until which adaptive fast-polling is active.
    // Set when a M ≥ adaptive_magnitude_threshold event is published; cleared
    // when the deadline passes.
    let mut fast_poll_until: Option<Instant> = None;

    loop {
        let since = Utc::now() - config.lookback_window;

        // A stalled source (e.g. USGS timing out) no longer blocks EMSC.
        // Each future captures a reference to its source; join_all drives them
        // all to completion before we proceed to the processing phase.
        let fetch_results = join_all(sources.iter().map(|source| async move {
            let poll_start = Instant::now();
            let result = source.fetch(since).await;
            (source.name(), poll_start, result)
        }))
        .await;

        for (source_name, poll_start, result) in fetch_results {
            match result {
                Ok(events) => {
                    let fetched = events.len();

                    metrics
                        .events_received_total
                        .with_label_values(&[source_name])
                        .inc_by(fetched as f64);

                    let mut published = 0usize;
                    let mut deduplicated = 0usize;
                    let mut rejected = 0usize;

                    for event in events {
                        if deduplicator.is_duplicate(&event.source_id).await {
                            deduplicated += 1;
                            metrics
                                .events_deduplicated_total
                                .with_label_values(&[source_name])
                                .inc();
                            continue;
                        }

                        if !deduplicator.is_redis_healthy() {
                            metrics
                                .dedup_redis_fallback_total
                                .with_label_values(&[source_name])
                                .inc();
                        }

                        let payload = match encoder.encode(&event) {
                            Ok(b) => b,
                            Err(e) => {
                                warn!(
                                    error = %e,
                                    source_id = event.source_id,
                                    "Avro encoding failed — routing to dead-letter"
                                );
                                let envelope = IngestDlqEnvelope {
                                    failure_reason: "avro_encode_error",
                                    failure_detail: e.to_string(),
                                    source_name,
                                    source_id: &event.source_id,
                                    raw_payload: &event.raw_payload,
                                };
                                if let Err(()) = dlq.send(&envelope).await {
                                    // DLQ failure is already logged at ERROR inside DlqProducer::send.
                                }
                                rejected += 1;
                                metrics
                                    .events_rejected_total
                                    .with_label_values(&[source_name, "avro_encode_error"])
                                    .inc();
                                continue;
                            }
                        };

                        match producer.send(&event.source_id, payload).await {
                            Ok(()) => {
                                deduplicator.mark_seen(&event.source_id).await;
                                published += 1;
                                metrics
                                    .events_published_total
                                    .with_label_values(&[source_name])
                                    .inc();

                                // Significant event: activate adaptive fast-polling
                                // window so aftershocks are captured quickly.
                                if event.magnitude >= config.adaptive_magnitude_threshold {
                                    let new_deadline = Instant::now() + config.adaptive_poll_window;
                                    // Extend the adaptive window on each significant event so that
                                    // aftershock sequences keep fast-poll active for the full window
                                    // duration after the last qualifying event. This can keep fast-poll
                                    // active indefinitely during highly active sequences — which is the
                                    // correct behaviour for seismic hazard monitoring.
                                    let is_new_or_extended =
                                        fast_poll_until.map_or(true, |t| new_deadline > t);
                                    if is_new_or_extended {
                                        fast_poll_until = Some(new_deadline);
                                        metrics
                                            .adaptive_poll_activations_total
                                            .with_label_values(&[source_name])
                                            .inc();
                                        info!(
                                            source = source_name,
                                            magnitude = event.magnitude,
                                            threshold = config.adaptive_magnitude_threshold,
                                            window_secs = config.adaptive_poll_window.as_secs(),
                                            "Adaptive fast-polling activated"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                error!(
                                    error = %e,
                                    source_id = event.source_id,
                                    "Kafka produce failed after retries — routing to dead-letter, event will retry on next poll cycle"
                                );
                                let envelope = IngestDlqEnvelope {
                                    failure_reason: "kafka_produce_error",
                                    failure_detail: e.to_string(),
                                    source_name,
                                    source_id: &event.source_id,
                                    raw_payload: &event.raw_payload,
                                };
                                if let Err(()) = dlq.send(&envelope).await {
                                    // DLQ failure is already logged at ERROR inside DlqProducer::send.
                                }
                                rejected += 1;
                                metrics
                                    .events_rejected_total
                                    .with_label_values(&[source_name, "kafka_produce_error"])
                                    .inc();
                            }
                        }
                    }

                    let elapsed = poll_start.elapsed().as_secs_f64();
                    metrics
                        .poll_duration_seconds
                        .with_label_values(&[source_name])
                        .observe(elapsed);

                    info!(
                        source = source_name,
                        fetched,
                        published,
                        deduplicated,
                        rejected,
                        elapsed_ms = (elapsed * 1000.0) as u64,
                        "Poll cycle complete",
                    );
                }
                Err(e) => {
                    error!(
                        source = source_name,
                        error = %e,
                        "Source fetch failed after retries — skipping this cycle"
                    );
                    metrics
                        .events_rejected_total
                        .with_label_values(&[source_name, "fetch_error"])
                        .inc();
                }
            }
        }

        // Use a shorter poll interval while the adaptive window is active.
        let sleep_duration: Duration = match fast_poll_until {
            Some(deadline) if deadline > Instant::now() => config.adaptive_poll_interval,
            _ => {
                fast_poll_until = None;
                config.poll_interval
            }
        };

        tokio::select! {
            _ = tokio::time::sleep(sleep_duration) => {}
            _ = shutdown.changed() => {
                info!("Poll loop received shutdown signal — exiting after current batch");
                break;
            }
        }
    }

    info!("Poll loop stopped");
}

async fn run_metrics_server(
    metrics: Arc<Metrics>,
    port: u16,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), error::IngestError> {
    use prometheus::Encoder;

    let metrics_ref = Arc::clone(&metrics);

    let app = Router::new()
        .route(
            "/metrics",
            get(move || {
                let metrics = Arc::clone(&metrics_ref);
                async move {
                    let encoder = prometheus::TextEncoder::new();
                    let mut buf = Vec::new();

                    match encoder.encode(&metrics.registry.gather(), &mut buf) {
                        Ok(()) => Response::builder()
                            .header(axum::http::header::CONTENT_TYPE, encoder.format_type())
                            .body(Body::from(buf))
                            .unwrap_or_else(|_| {
                                // Builder with a literal 500 status and empty body is
                                // always valid; the inner unwrap_or_else is unreachable.
                                Response::builder()
                                    .status(500)
                                    .body(Body::empty())
                                    .unwrap_or_else(|_| Response::new(Body::empty()))
                            }),
                        Err(e) => {
                            error!(error = %e, "Failed to encode Prometheus metrics");
                            Response::builder()
                                .status(500)
                                .body(Body::from(e.to_string()))
                                .unwrap_or_else(|_| Response::new(Body::empty()))
                        }
                    }
                }
            }),
        )
        .route("/health", get(|| async { "ok" }));

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| error::IngestError::HttpServer(e.to_string()))?;

    info!(port, "Metrics server listening on /metrics and /health");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            // Resolves when the shutdown flag is set to true, or when the
            // sender is dropped (service exit). In either case, stop accepting
            // new connections and drain in-flight requests.
            let _ = shutdown.wait_for(|v| *v).await;
        })
        .await
        .map_err(|e| error::IngestError::HttpServer(e.to_string()))
}

/// Resolves on SIGTERM (Unix) or Ctrl-C (all platforms).
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
