use std::{net::SocketAddr, sync::Arc, time::Duration};

use tracing::{error, info};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod cache;
mod config;
mod db;
mod error;
mod metrics;
mod model;
mod openapi;
mod routes;

use cache::Cache;
use config::Config;
use metrics::Metrics;
use routes::{build_router, AppState};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json())
        .init();

    info!("seismosis-api starting");

    let config = Config::from_env().map_err(|e| anyhow::anyhow!("config: {}", e))?;
    info!(
        http_port = config.http_port,
        request_timeout_secs = config.request_timeout_secs,
        "Config loaded"
    );

    // `acquire_timeout` prevents indefinite stalls when the pool is exhausted.
    // Requests that cannot obtain a connection within 5 s will receive a 500,
    // which is preferable to hanging until the client times out.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&config.database_url)
        .await
        .map_err(|e| anyhow::anyhow!("db pool: {}", e))?;
    info!("Database pool established");

    let redis_client = redis::Client::open(config.redis_url.as_str())
        .map_err(|e| anyhow::anyhow!("redis client: {}", e))?;
    let redis_conn = redis::aio::ConnectionManager::new(redis_client)
        .await
        .map_err(|e| anyhow::anyhow!("redis connection manager: {}", e))?;
    info!("Redis connection manager established");

    let prom_registry = prometheus::Registry::new();
    let metrics =
        Arc::new(Metrics::new(&prom_registry).map_err(|e| anyhow::anyhow!("metrics: {}", e))?);
    let prom_registry = Arc::new(prom_registry);

    let state = AppState {
        pool,
        cache: Cache::new(redis_conn),
        metrics,
        config: Arc::new(config.clone()),
        prom_registry,
    };

    let app = build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.http_port));
    info!(addr = %addr, "API server listening");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("bind {}: {}", addr, e))?;

    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            match tokio::signal::ctrl_c().await {
                Ok(()) => info!("Received Ctrl-C, initiating graceful shutdown"),
                Err(e) => error!(error = %e, "Failed to listen for shutdown signal"),
            }
        })
        .await
    {
        error!(error = %e, "HTTP server error");
        return Err(anyhow::anyhow!("server: {}", e));
    }

    info!("seismosis-api stopped");
    Ok(())
}
