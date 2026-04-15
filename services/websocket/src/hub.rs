//! Client hub — registry of connected WebSocket clients and fan-out logic.
//!
//! The hub is an `Arc<Hub>` shared between:
//!   - The Kafka consumer task: calls `broadcast_enriched` / `broadcast_alert`.
//!   - The WebSocket accept loop: calls `register` / `remove`.
//!   - Each per-client task: calls `update_filter` and `remove`.
//!
//! ## Concurrency model
//!
//! The client map uses a `tokio::sync::RwLock` — multiple reader tasks (the
//! consumer fan-out) can hold concurrent read locks, while writer operations
//! (register / remove / update_filter) take an exclusive write lock.
//!
//! Fan-out holds a *read* lock for its entire duration; individual channel
//! sends are non-blocking (`try_send`) so slow clients cannot stall the lock.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, RwLock};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::event::{AlertEvent, EnrichedEvent, ServerMessage};
use crate::filter::SubscriptionFilter;
use crate::metrics::Metrics;

// ─── Internal state ───────────────────────────────────────────────────────────

struct ClientEntry {
    filter: SubscriptionFilter,
    tx: mpsc::Sender<Arc<ServerMessage>>,
}

// ─── Hub ──────────────────────────────────────────────────────────────────────

pub struct Hub {
    clients: RwLock<HashMap<Uuid, ClientEntry>>,
    metrics: Arc<Metrics>,
    channel_capacity: usize,
}

impl Hub {
    pub fn new(metrics: Arc<Metrics>, channel_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            clients: RwLock::new(HashMap::new()),
            metrics,
            channel_capacity,
        })
    }

    /// Register a new client with the given subscription filter.
    ///
    /// Returns the receiving end of the per-client message channel.
    /// The sender end is stored in the hub; the receiver is owned by the
    /// per-client task.
    pub async fn register(
        &self,
        id: Uuid,
        filter: SubscriptionFilter,
    ) -> mpsc::Receiver<Arc<ServerMessage>> {
        let (tx, rx) = mpsc::channel(self.channel_capacity);
        let mut guard = self.clients.write().await;
        guard.insert(id, ClientEntry { filter, tx });
        self.metrics.clients_connected.inc();
        debug!(client_id = %id, min_magnitude = ?guard[&id].filter.min_magnitude, "Client registered");
        rx
    }

    /// Remove a client from the registry.
    ///
    /// Called by the per-client task on any exit path (clean close, error,
    /// or shutdown).
    pub async fn remove(&self, id: Uuid) {
        let mut guard = self.clients.write().await;
        if guard.remove(&id).is_some() {
            self.metrics.clients_connected.dec();
            debug!(client_id = %id, "Client removed");
        }
    }

    /// Replace the subscription filter for a connected client.
    ///
    /// No-op if `id` is no longer in the registry (client already removed).
    pub async fn update_filter(&self, id: Uuid, filter: SubscriptionFilter) {
        let mut guard = self.clients.write().await;
        if let Some(entry) = guard.get_mut(&id) {
            entry.filter = filter;
            debug!(client_id = %id, "Subscription filter updated");
        }
    }

    /// Fan-out an enriched event to all clients whose filter matches.
    ///
    /// The event is heap-allocated once and shared across all matching clients
    /// via `Arc` clones — no per-client copies of the event data are made.
    pub async fn broadcast_enriched(&self, event: EnrichedEvent) {
        let msg = Arc::new(ServerMessage::Earthquake(Box::new(event)));
        // Extract the inner event reference before the read lock so the loop
        // body can call matches_enriched directly without a per-iteration
        // `if let` on the known variant.
        let ServerMessage::Earthquake(ref enriched) = *msg else {
            unreachable!("msg was just constructed as Earthquake")
        };
        let guard = self.clients.read().await;

        for (id, entry) in guard.iter() {
            if entry.filter.matches_enriched(enriched) {
                self.try_send(*id, &entry.tx, Arc::clone(&msg), "enriched");
            } else {
                self.metrics
                    .messages_filtered_total
                    .with_label_values(&["enriched"])
                    .inc();
            }
        }

        self.metrics
            .messages_consumed_total
            .with_label_values(&["enriched"])
            .inc();
    }

    /// Fan-out an alert event to all clients whose filter matches.
    pub async fn broadcast_alert(&self, event: AlertEvent) {
        let msg = Arc::new(ServerMessage::Alert(event));
        // Extract the inner event reference before the read lock (same pattern
        // as broadcast_enriched — avoids a fragile inner `if let` in the loop).
        let ServerMessage::Alert(ref alerted) = *msg else {
            unreachable!("msg was just constructed as Alert")
        };
        let guard = self.clients.read().await;

        for (id, entry) in guard.iter() {
            if entry.filter.matches_alert(alerted) {
                self.try_send(*id, &entry.tx, Arc::clone(&msg), "alert");
            } else {
                self.metrics
                    .messages_filtered_total
                    .with_label_values(&["alert"])
                    .inc();
            }
        }

        self.metrics
            .messages_consumed_total
            .with_label_values(&["alert"])
            .inc();
    }

    /// Send a `Close` signal to all connected clients.
    ///
    /// Called once during graceful shutdown.  The per-client tasks will
    /// detect the `Close` variant and send a WebSocket Close frame before
    /// exiting.
    pub async fn close_all(&self) {
        let guard = self.clients.read().await;
        let close = Arc::new(ServerMessage::Close);
        for (id, entry) in guard.iter() {
            if entry.tx.try_send(Arc::clone(&close)).is_err() {
                // Channel full or receiver dropped — the client task is
                // already exiting; no action needed.
                debug!(client_id = %id, "Could not send Close to client (already closing)");
            }
        }
    }

    /// Non-blocking send to a single client's channel.
    ///
    /// Logs a warning and increments the drop counter when the channel is full.
    fn try_send(
        &self,
        id: Uuid,
        tx: &mpsc::Sender<Arc<ServerMessage>>,
        msg: Arc<ServerMessage>,
        topic_label: &str,
    ) {
        match tx.try_send(msg) {
            Ok(()) => {
                self.metrics
                    .messages_delivered_total
                    .with_label_values(&[topic_label])
                    .inc();
            }
            Err(_) => {
                warn!(client_id = %id, topic = topic_label, "Client channel full — dropping message");
                self.metrics
                    .messages_dropped_total
                    .with_label_values(&[topic_label])
                    .inc();
            }
        }
    }
}
