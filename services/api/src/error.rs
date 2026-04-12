use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use thiserror::Error;

/// Errors that prevent the service from starting.
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("config: {0}")]
    Config(String),

    #[error("database: {0}")]
    Database(#[from] sqlx::Error),

    #[error("redis: {0}")]
    Redis(String),

    #[error("metrics: {0}")]
    Metrics(String),
}

/// Per-request errors returned as JSON bodies.
///
/// The `IntoResponse` implementation maps each variant to an appropriate
/// HTTP status code. 5xx errors are logged at ERROR before the request
/// handler returns; 4xx errors are expected application conditions and
/// should not be logged as errors.
#[derive(Debug, Error)]
pub enum RequestError {
    #[error("not found")]
    NotFound,

    #[error("invalid query parameter '{param}': {detail}")]
    BadParam { param: &'static str, detail: String },

    #[error("database error")]
    Database(#[from] sqlx::Error),

    #[error("cache error")]
    Cache(String),
}

/// JSON body for all error responses.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl IntoResponse for RequestError {
    fn into_response(self) -> axum::response::Response {
        let (status, body) = match &self {
            RequestError::NotFound => (
                StatusCode::NOT_FOUND,
                ErrorBody {
                    error: "not_found".to_owned(),
                    detail: None,
                },
            ),
            RequestError::BadParam { param, detail } => (
                StatusCode::BAD_REQUEST,
                ErrorBody {
                    error: "bad_param".to_owned(),
                    detail: Some(format!("{}: {}", param, detail)),
                },
            ),
            RequestError::Database(e) => {
                tracing::error!(error = %e, "Database error in request handler");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorBody {
                        error: "internal_error".to_owned(),
                        detail: None,
                    },
                )
            }
            RequestError::Cache(e) => {
                tracing::warn!(error = %e, "Cache error in request handler");
                // Cache errors degrade gracefully; we still serve from DB.
                // This variant is only returned when degradation isn't possible.
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorBody {
                        error: "internal_error".to_owned(),
                        detail: None,
                    },
                )
            }
        };

        (status, Json(body)).into_response()
    }
}
