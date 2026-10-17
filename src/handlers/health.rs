//! Health check endpoints.
use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::models::responses::HealthResponse;
use crate::state::AppState;
use crate::utils::formatting::{current_timestamp, format_uptime};

pub async fn root() -> &'static str {
    "GitHub Version API - Running ✅"
}

pub async fn health_check(State(state): State<Arc<AppState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        version: env!("CARGO_PKG_VERSION"),
        uptime: format_uptime(state.started_at),
        timestamp: current_timestamp(),
    })
}
