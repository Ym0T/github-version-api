//! HTTP handlers.
pub mod diff;
pub mod download;
pub mod health;
pub mod stats;
pub mod version;

use crate::error::ApiError;

/// Fallback for unknown routes.
pub async fn not_found() -> ApiError {
    ApiError::not_found("not_found", "No such endpoint")
}

/// Fallback for unsupported methods on known routes.
pub async fn method_not_allowed() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "Only GET requests are supported",
    )
}
