use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{routing::get, Router};
use prometheus::{Encoder, Registry, TextEncoder};
use rdkafka::{
    config::ClientConfig,
    consumer::{CommitMode, Consumer, StreamConsumer},
    message::Message,
};
use tokio::sync::watch;
use tracing::{error, info, warn};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod avro;
mod config;
mod db;
mod dead_letter;
mod error;
mod metrics;
mod model;
mod registry;

use avro::AvroDecoder;
use config::Config;
use dead_letter::DlqProducer;
use metrics::Metrics;
use registry::SchemaRegistryClient;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json())
        .init();

    info!("seismosis-storage starting");

    let config = Config::from_env().map_err(|e| anyhow::anyhow!("config: {}", e))?;
    info!(
        kafka_brokers = %config.kafka_brokers,
        kafka_topic_raw = %config.kafka_topic_raw,
        kafka_group_id = %config.kafka_group_id,
        metrics_port = config.metrics_port,
        "Config loaded"
    );

    let pool = db::create_pool(&config)
        .await
        .map_err(|e| anyhow::anyhow!("db: {}", e))?;
    info!("Database pool established");

    // Optional Redis connection for API cache invalidation. Startup continues
    // if Redis is unavailable — cache entries will expire naturally on their TTL.
    let redis_conn: Option<redis::aio::ConnectionManager> = match &config.redis_url {
        None => {
            info!("REDIS_URL not set — cache invalidation disabled");
            None
        }
        Some(url) => match redis::Client::open(url.as_str()) {
            Err(e) => {
                warn!(error = %e, "Redis client creation failed — cache invalidation disabled");
                None
            }
            Ok(client) => match redis::aio::ConnectionManager::new(client).await {
                Ok(mgr) => {
                    info!("Redis connection established for cache invalidation");
                    Some(mgr)
                }
                Err(e) => {
                    warn!(error = %e, "Redis connection failed — cache invalidation disabled");
                    None
                }
            },
        },
    };

    let registry_client = SchemaRegistryClient::new(config.schema_registry_url.clone())
        .map_err(|e| anyhow::anyhow!("schema registry: {}", e))?;
    let decoder = Arc::new(AvroDecoder::new(registry_client));

    let prom_registry = Registry::new();
    let metrics =
        Arc::new(Metrics::new(&prom_registry).map_err(|e| anyhow::anyhow!("metrics: {}", e))?);

    let dlq = DlqProducer::new(
        &config.kafka_brokers,
        config.kafka_topic_dead_letter.clone(),
    )
    .map_err(|e| anyhow::anyhow!("dlq producer: {}", e))?;

    let consumer: StreamConsumer = ClientConfig::new()
        .set("bootstrap.servers", &config.kafka_brokers)
        .set("group.id", &config.kafka_group_id)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("session.timeout.ms", "10000")
        .set("heartbeat.interval.ms", "3000")
        // 5-minute poll interval. The worst-case per-message processing time is
        // ~8 s (DLQ: 4 attempts × up to 2 s sleep + 5 s delivery timeout), so
        // this value provides a very wide safety margin and will never be hit.
        .set("max.poll.interval.ms", "300000")
        .set("fetch.min.bytes", "1") // respond immediately on any available data
        .set("fetch.wait.max.ms", "100") // reduce idle poll latency
        .create()
        .map_err(|e| anyhow::anyhow!("kafka consumer: {}", e))?;

    consumer
        .subscribe(&[&config.kafka_topic_raw])
        .map_err(|e| anyhow::anyhow!("kafka subscribe: {}", e))?;

    info!(topic = %config.kafka_topic_raw, "Subscribed to topic");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let metrics_handle = {
        let shutdown_rx = shutdown_rx.clone();
        tokio::spawn(run_metrics_server(
            prom_registry,
            config.metrics_port,
            shutdown_rx,
        ))
    };

    let consume_handle = {
        let metrics = Arc::clone(&metrics);
        let topic = config.kafka_topic_raw.clone();
        tokio::spawn(async move {
            let ctx = ConsumeCtx {
                consumer,
                decoder,
                pool,
                dlq,
                metrics,
                topic,
                redis_conn,
            };
            consume_loop(ctx, shutdown_rx).await;
        })
    };

    match tokio::signal::ctrl_c().await {
        Ok(()) => info!("Received Ctrl-C, initiating graceful shutdown"),
        Err(e) => error!(error = %e, "Failed to listen for shutdown signal"),
    }

    // Signal both tasks to stop. `send` returns Err only when every Receiver
    // has been dropped; both tasks still hold theirs at this point, so the
    // discard is safe and the `let _` is intentional.
    let _ = shutdown_tx.send(true); // intentional — see comment above

    if let Err(e) = consume_handle.await {
        error!(error = %e, "consume_loop task panicked");
    }
    if let Err(e) = metrics_handle.await {
        error!(error = %e, "metrics_server task panicked");
    }

    info!("seismosis-storage stopped");
    Ok(())
}

struct ConsumeCtx {
    consumer: StreamConsumer,
    decoder: Arc<AvroDecoder>,
    pool: sqlx::PgPool,
    dlq: DlqProducer,
    metrics: Arc<Metrics>,
    topic: String,
    redis_conn: Option<redis::aio::ConnectionManager>,
}

async fn consume_loop(ctx: ConsumeCtx, mut shutdown: watch::Receiver<bool>) {
    let ConsumeCtx {
        consumer,
        decoder,
        pool,
        dlq,
        metrics,
        topic,
        redis_conn,
    } = ctx;
    loop {
        if *shutdown.borrow() {
            break;
        }

        let msg = tokio::select! {
            biased;
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
                continue;
            }
            result = consumer.recv() => match result {
                Ok(m) => m,
                Err(e) => {
                    warn!(error = %e, "Kafka consumer error");
                    // Sleep before the next recv() attempt to avoid a tight spin
                    // under sustained broker unavailability. 500 ms is short
                    // enough not to materially delay graceful shutdown.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
            }
        };

        let partition = msg.partition();
        let offset = msg.offset();

        // A null payload is a Kafka tombstone (used in log-compacted topics).
        // `earthquakes.raw` is not compacted, so this should never occur in
        // normal operation. Skip and commit rather than routing to the DLQ as
        // a decode error, since there is no message body to dead-letter.
        let payload = match msg.payload() {
            Some(p) => p,
            None => {
                warn!(
                    partition,
                    offset, "Received message with null payload — skipping"
                );
                if let Err(e) = consumer.commit_message(&msg, CommitMode::Async) {
                    warn!(error = %e, partition, offset, "Offset commit failed");
                }
                continue;
            }
        };

        metrics
            .messages_consumed_total
            .with_label_values(&[&topic])
            .inc();

        let msg_start = Instant::now();
        let process_result =
            process_message(payload, &decoder, &pool, &metrics, redis_conn.clone()).await;

        // Observe immediately after process_message — captures only decode +
        // validate + upsert time. DLQ delivery (up to ~7.5 s under full retry)
        // is excluded so error-path latency does not inflate the p99.
        metrics
            .processing_duration_seconds
            .with_label_values(&[&topic])
            .observe(msg_start.elapsed().as_secs_f64());

        // `should_commit` gates offset acknowledgment:
        //   true  — upsert succeeded, or processing failed and DLQ accepted it.
        //   false — DLQ delivery failed; leave the offset uncommitted so the
        //           message is redelivered after consumer restart. The upsert is
        //           idempotent, so a reprocessed message that succeeds next time
        //           is safe.
        let should_commit = match process_result {
            Ok(()) => true,
            Err(err) => {
                // For wire/decode failures we have no decoded key; fall back to
                // the raw Kafka message key set by the ingestion producer.
                let source_id = msg.key().and_then(|k| std::str::from_utf8(k).ok());

                metrics
                    .events_dead_letter_total
                    .with_label_values(&[err.failure_reason()])
                    .inc();

                warn!(
                    failure_reason = err.failure_reason(),
                    source_id,
                    partition,
                    offset,
                    error = %err,
                    "Processing failed — routing to dead-letter"
                );

                match dlq
                    .send(source_id, payload, &err, &topic, partition, offset)
                    .await
                {
                    Ok(()) => true,
                    Err(()) => {
                        // DLQ exhausted retries. The error is already logged at
                        // ERROR in DlqProducer::send. Do not commit — the
                        // message will be reprocessed on the next startup.
                        warn!(
                            partition,
                            offset, "DLQ delivery failed — offset not committed"
                        );
                        metrics
                            .events_dlq_delivery_failed_total
                            .with_label_values(&[&topic])
                            .inc();
                        false
                    }
                }
            }
        };

        if should_commit {
            if let Err(e) = consumer.commit_message(&msg, CommitMode::Async) {
                warn!(error = %e, partition, offset, "Offset commit failed");
            }
        }
    }

    info!("Consume loop exiting");
}

async fn process_message(
    payload: &[u8],
    decoder: &AvroDecoder,
    pool: &sqlx::PgPool,
    metrics: &Metrics,
    redis: Option<redis::aio::ConnectionManager>,
) -> Result<(), error::ProcessError> {
    let raw = decoder.decode(payload).await?;
    let event = raw.validate()?;

    // Cross-source deduplication: if an event from a different source network
    // already exists within 30 s and 0.5° of this one, skip the lower-quality
    // duplicate rather than storing two records for the same physical earthquake.
    if let Some(existing) = db::find_cross_source_match(pool, &event).await? {
        let incoming_rank = db::quality_rank(&event.quality_indicator);
        let existing_rank = db::quality_rank(&existing.quality_indicator);

        if incoming_rank <= existing_rank {
            metrics
                .cross_source_duplicates_total
                .with_label_values(&[&event.source_network])
                .inc();
            tracing::debug!(
                incoming_source_id = %event.source_id,
                existing_source_id = %existing.source_id,
                incoming_quality = %event.quality_indicator,
                existing_quality = %existing.quality_indicator,
                "Cross-source duplicate — skipping lower-quality event"
            );
            return Ok(());
        }
    }

    db::upsert_event(pool, &event, metrics, redis).await?;

    Ok(())
}

async fn run_metrics_server(registry: Registry, port: u16, mut shutdown: watch::Receiver<bool>) {
    let registry = Arc::new(registry);

    let app = Router::new().route(
        "/metrics",
        get(move || {
            let registry = Arc::clone(&registry);
            async move {
                let encoder = TextEncoder::new();
                let mut buf = Vec::new();
                if let Err(e) = encoder.encode(&registry.gather(), &mut buf) {
                    error!(error = %e, "Failed to encode metrics");
                }
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        encoder.format_type().to_owned(),
                    )],
                    buf,
                )
            }
        }),
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!(addr = %addr, "Metrics server listening");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!(error = %e, addr = %addr, "Failed to bind metrics server");
            return;
        }
    };

    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown.wait_for(|v| *v).await;
        })
        .await
    {
        error!(error = %e, "Metrics server error");
    }
}
