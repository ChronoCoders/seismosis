//! WebSocket connection handler.
//!
//! `handle_connection` is called once per accepted TCP connection.  It:
//!
//! 1. Upgrades the TCP stream to a WebSocket via `accept_hdr_async`, capturing
//!    subscription filter parameters from the HTTP upgrade request's query string.
//! 2. Registers the client with the `Hub` and receives the outbound channel.
//! 3. Sends a `"connected"` welcome message with the assigned client ID.
//! 4. Drives a select loop across three arms:
//!    - **Outbound**: reads from the per-client channel and writes JSON to the socket.
//!    - **Inbound**: reads control messages from the client (filter updates, ping/pong).
//!    - **Shutdown**: closes the socket with a 1001 Going Away frame on signal.
//! 5. Removes the client from the hub on any exit path.

use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{Request, Response},
        protocol::{frame::coding::CloseCode, CloseFrame},
        Message as WsMessage,
    },
};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::event::ServerMessage;
use crate::filter::SubscriptionFilter;
use crate::hub::Hub;
use crate::metrics::Metrics;

// ─── Connection handler ───────────────────────────────────────────────────────

pub async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    hub: Arc<Hub>,
    metrics: Arc<Metrics>,
    mut shutdown: watch::Receiver<bool>,
    default_min_magnitude: f64,
) {
    // Capture the HTTP upgrade request's query string.  The `accept_hdr_async`
    // callback fires synchronously during the handshake (before the future
    // resolves), so a stack-allocated Mutex is sufficient — no Arc needed.
    let captured_query = std::sync::Mutex::new(String::new());
    let ws_result = accept_hdr_async(stream, |req: &Request, resp: Response| {
        *captured_query.lock().unwrap_or_else(|e| e.into_inner()) =
            req.uri().query().unwrap_or("").to_owned();
        Ok(resp)
    })
    .await;

    let ws_stream = match ws_result {
        Ok(ws) => ws,
        Err(e) => {
            warn!(peer = %addr, error = %e, "WebSocket handshake failed");
            return;
        }
    };

    // After the await the future (and the closure it held) are gone; the
    // Mutex is no longer borrowed and into_inner() can move out the String.
    let query = captured_query.into_inner().unwrap_or_default();
    let filter = SubscriptionFilter::from_query(&query, default_min_magnitude);

    let client_id = Uuid::new_v4();
    info!(
        client_id = %client_id,
        peer = %addr,
        min_magnitude = filter.min_magnitude,
        "WebSocket client connected",
    );

    // Classify the remote address coarsely for the counter label.
    let addr_class = if addr.ip().is_loopback() { "internal" } else { "external" };
    metrics
        .connections_accepted_total
        .with_label_values(&[addr_class])
        .inc();

    let mut rx = hub.register(client_id, filter).await;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Send the welcome message before entering the event loop.
    let welcome = serde_json::json!({
        "type": "connected",
        "client_id": client_id.to_string(),
    });
    if ws_write
        .send(WsMessage::Text(welcome.to_string()))
        .await
        .is_err()
    {
        hub.remove(client_id).await;
        metrics
            .connections_closed_total
            .with_label_values(&["error"])
            .inc();
        return;
    }

    // Guard: watch::changed() only fires on value transitions.  If shutdown
    // was already set before this task starts (possible when a client connects
    // in the shutdown window), the select loop would never see it.
    if *shutdown.borrow() {
        let _ = ws_write
            .send(WsMessage::Close(Some(CloseFrame {
                code: CloseCode::Away,
                reason: "server shutting down".into(),
            })))
            .await;
        hub.remove(client_id).await;
        metrics
            .connections_closed_total
            .with_label_values(&["server_shutdown"])
            .inc();
        info!(
            client_id = %client_id,
            peer = %addr,
            "Client connected during shutdown window — closing immediately",
        );
        return;
    }

    // ── Main event loop ───────────────────────────────────────────────────
    let close_reason = loop {
        tokio::select! {
            biased;

            // ── Shutdown signal ───────────────────────────────────────────
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    let _ = ws_write
                        .send(WsMessage::Close(Some(CloseFrame {
                            code: CloseCode::Away,
                            reason: "server shutting down".into(),
                        })))
                        .await;
                    break "server_shutdown";
                }
            }

            // ── Outbound: hub → client ────────────────────────────────────
            msg = rx.recv() => {
                match msg {
                    None => {
                        // Hub dropped the sender — service is shutting down.
                        break "server_shutdown";
                    }
                    Some(server_msg) => {
                        match server_msg.as_ref() {
                            ServerMessage::Close => {
                                let _ = ws_write
                                    .send(WsMessage::Close(Some(CloseFrame {
                                        code: CloseCode::Away,
                                        reason: "server shutting down".into(),
                                    })))
                                    .await;
                                break "server_shutdown";
                            }
                            other => {
                                match other.to_json() {
                                    Some(json) => {
                                        if ws_write.send(WsMessage::Text(json)).await.is_err() {
                                            break "error";
                                        }
                                    }
                                    None => {
                                        // to_json() returns None only for Close, which is
                                        // handled above.  A None here means a new
                                        // ServerMessage variant was added without updating
                                        // to_json() — log so it surfaces in structured logs.
                                        warn!(
                                            client_id = %client_id,
                                            "to_json() returned None for non-Close message — skipping"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Inbound: client → hub ─────────────────────────────────────
            incoming = ws_read.next() => {
                match incoming {
                    None | Some(Err(_)) => break "client_close",
                    Some(Ok(WsMessage::Close(_))) => break "client_close",
                    Some(Ok(WsMessage::Ping(data))) => {
                        if ws_write.send(WsMessage::Pong(data)).await.is_err() {
                            break "error";
                        }
                    }
                    Some(Ok(WsMessage::Text(text))) => {
                        handle_client_message(&text, client_id, &hub).await;
                    }
                    Some(Ok(_)) => {} // ignore Binary, Pong, Frame fragments
                }
            }
        }
    };

    hub.remove(client_id).await;
    metrics
        .connections_closed_total
        .with_label_values(&[close_reason])
        .inc();

    info!(
        client_id = %client_id,
        peer = %addr,
        reason = close_reason,
        "WebSocket client disconnected",
    );
}

// ─── Client → server control messages ────────────────────────────────────────

async fn handle_client_message(text: &str, client_id: Uuid, hub: &Hub) {
    #[derive(serde::Deserialize)]
    struct ClientMsg {
        #[serde(rename = "type")]
        msg_type: String,
        #[serde(default)]
        min_magnitude: Option<f64>,
        #[serde(default)]
        lat_min: Option<f64>,
        #[serde(default)]
        lat_max: Option<f64>,
        #[serde(default)]
        lon_min: Option<f64>,
        #[serde(default)]
        lon_max: Option<f64>,
        #[serde(default)]
        source_networks: Option<Vec<String>>,
    }

    let msg = match serde_json::from_str::<ClientMsg>(text) {
        Ok(m) => m,
        Err(_) => {
            debug!(client_id = %client_id, "Ignoring unparseable client message");
            return;
        }
    };

    if msg.msg_type == "subscribe" {
        let new_filter = SubscriptionFilter {
            min_magnitude: msg.min_magnitude.unwrap_or(0.0),
            lat_min: msg.lat_min,
            lat_max: msg.lat_max,
            lon_min: msg.lon_min,
            lon_max: msg.lon_max,
            source_networks: msg
                .source_networks
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.to_uppercase())
                .collect(),
        };
        hub.update_filter(client_id, new_filter).await;
        debug!(client_id = %client_id, "Subscription filter updated via control message");
    }
}
