use prometheus::{CounterVec, Gauge, HistogramOpts, HistogramVec, Opts, Registry};
use std::sync::Arc;

/// All Prometheus metrics for the WebSocket service.
///
/// Naming convention: `seismosis_websocket_{metric}_{unit}`.
pub struct Metrics {
    /// Current number of WebSocket clients connected.
    pub clients_connected: Gauge,

    /// Cumulative WebSocket connections accepted (monotonically increasing).
    pub connections_accepted_total: CounterVec,

    /// Cumulative WebSocket connections that have closed.
    pub connections_closed_total: CounterVec,

    /// Kafka messages consumed from the upstream topics (before fan-out).
    /// `topic` label: "enriched" | "alert"
    pub messages_consumed_total: CounterVec,

    /// Individual (message × client) deliveries where the filter matched and
    /// the message was queued to the client's outbound channel.
    /// `topic` label: "enriched" | "alert"
    pub messages_delivered_total: CounterVec,

    /// Individual (message × client) pairs where the subscription filter
    /// excluded the message for that client.
    /// `topic` label: "enriched" | "alert"
    pub messages_filtered_total: CounterVec,

    /// Messages dropped because a client's outbound channel was full.
    pub messages_dropped_total: CounterVec,

    /// End-to-end duration of the broadcast fan-out for one Kafka message
    /// (from message received to all channel sends attempted).
    /// `topic` label: "enriched" | "alert"
    pub broadcast_duration_seconds: HistogramVec,

    /// Backing registry — used by the metrics HTTP handler.
    pub registry: Registry,
}

impl Metrics {
    pub fn new() -> Result<Arc<Self>, prometheus::Error> {
        let registry = Registry::new_custom(None, None)?;

        macro_rules! register {
            ($metric:expr) => {{
                let m = $metric;
                registry.register(Box::new(m.clone()))?;
                m
            }};
        }

        let clients_connected = register!(Gauge::with_opts(Opts::new(
            "seismosis_websocket_clients_connected",
            "Current number of WebSocket clients connected",
        ))?);

        let connections_accepted_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_websocket_connections_accepted_total",
                "Cumulative WebSocket connections accepted",
            ),
            &["remote_addr_class"], // "internal" | "external" — coarse, not per-IP
        )?);

        let connections_closed_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_websocket_connections_closed_total",
                "Cumulative WebSocket connections closed",
            ),
            &["reason"], // "client_close" | "server_shutdown" | "error"
        )?);

        let messages_consumed_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_websocket_messages_consumed_total",
                "Kafka messages consumed from upstream topics",
            ),
            &["topic"],
        )?);

        let messages_delivered_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_websocket_messages_delivered_total",
                "Individual client deliveries where filter matched (message queued to channel)",
            ),
            &["topic"],
        )?);

        let messages_filtered_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_websocket_messages_filtered_total",
                "Individual client skips where subscription filter excluded the message",
            ),
            &["topic"],
        )?);

        let messages_dropped_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_websocket_messages_dropped_total",
                "Messages dropped because a client outbound channel was full",
            ),
            &["topic"],
        )?);

        let broadcast_duration_seconds = register!(HistogramVec::new(
            HistogramOpts::new(
                "seismosis_websocket_broadcast_duration_seconds",
                "End-to-end fan-out duration per Kafka message (decode → all channel sends)",
            )
            .buckets(vec![
                0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25,
            ]),
            &["topic"],
        )?);

        Ok(Arc::new(Self {
            clients_connected,
            connections_accepted_total,
            connections_closed_total,
            messages_consumed_total,
            messages_delivered_total,
            messages_filtered_total,
            messages_dropped_total,
            broadcast_duration_seconds,
            registry,
        }))
    }
}
