//! Usage statistics endpoint.
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::state::AppState;
use crate::utils::formatting::{format_file_size, format_uptime};

#[derive(Serialize)]
pub struct StatsResponse {
    pub requests: RequestStats,
    pub cache: CacheStats,
    pub uptime: String,
    pub uptime_seconds: u64,
}

#[derive(Serialize)]
pub struct RequestStats {
    /// Lifetime total, persisted across restarts.
    pub total: u64,
    /// Average since the last restart.
    pub per_minute: f64,
}

#[derive(Serialize)]
pub struct CacheStats {
    pub hit_rate: f64,
    pub hits: u64,
    pub misses: u64,
    pub version_entries: u64,
    pub diff_entries: u64,
    pub memory: String,
}

pub async fn get_stats(State(state): State<Arc<AppState>>) -> Json<StatsResponse> {
    let snapshot = state.stats.snapshot();
    let uptime_seconds = state.started_at.elapsed().as_secs();
    let per_minute = if uptime_seconds > 0 {
        (snapshot.session_requests as f64 / uptime_seconds as f64 * 60.0 * 100.0).round() / 100.0
    } else {
        0.0
    };
    // Entry counts are only exact after pending cache maintenance has run.
    state.caches.versions.run_pending_tasks().await;
    state.caches.diffs.run_pending_tasks().await;
    let version_entries = state.caches.versions.entry_count();
    let diff_entries = state.caches.diffs.entry_count();

    Json(StatsResponse {
        requests: RequestStats {
            total: snapshot.total_requests,
            per_minute,
        },
        cache: CacheStats {
            hit_rate: snapshot.hit_rate(),
            hits: snapshot.cache_hits,
            misses: snapshot.cache_misses,
            version_entries,
            diff_entries,
            // Rough estimate: ~2 KiB per version entry plus the weighted diff cache.
            memory: format_file_size(version_entries * 2048 + state.caches.diffs.weighted_size()),
        },
        uptime: format_uptime(state.started_at),
        uptime_seconds,
    })
}
