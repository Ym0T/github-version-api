//! Persistence of the lifetime counters.
//!
//! Saves happen periodically and on shutdown. They are serialized and the
//! snapshot is taken inside the lock, so the newest counters always win.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::state::AppState;
use crate::utils::fs::{atomic_write, read_limited};
use crate::utils::stats::Stats;

const MAX_STATS_FILE_BYTES: u64 = 64 * 1024;

static SAVE_LOCK: Mutex<()> = Mutex::const_new(());

/// On-disk format. New fields need a default, so existing files keep loading.
#[derive(Debug, Serialize, Deserialize)]
struct PersistentStatsData {
    total_requests: u64,
    cache_hits: u64,
    cache_misses: u64,
    #[serde(default)]
    last_saved: String,
    #[serde(default)]
    version: String,
}

/// Loads saved counters; starts from zero if the file is missing or invalid.
pub async fn load(path: &Path) -> Stats {
    let data = match read_limited(path, MAX_STATS_FILE_BYTES).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Stats::default(),
        Err(e) => {
            warn!("cannot read stats file {}: {e}", path.display());
            return Stats::default();
        }
    };
    match serde_json::from_slice::<PersistentStatsData>(&data) {
        Ok(saved) => {
            info!(
                total_requests = saved.total_requests,
                "loaded persistent statistics"
            );
            Stats::with_totals(saved.total_requests, saved.cache_hits, saved.cache_misses)
        }
        Err(e) => {
            warn!("ignoring invalid stats file {}: {e}", path.display());
            Stats::default()
        }
    }
}

pub async fn save(stats: &Stats, path: &Path) -> anyhow::Result<()> {
    let _saving = SAVE_LOCK.lock().await;
    let snapshot = stats.snapshot();
    let data = PersistentStatsData {
        total_requests: snapshot.total_requests,
        cache_hits: snapshot.cache_hits,
        cache_misses: snapshot.cache_misses,
        last_saved: chrono::Utc::now().to_rfc3339(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    };
    atomic_write(path, serde_json::to_vec_pretty(&data)?).await?;
    Ok(())
}

pub fn spawn_saver(state: Arc<AppState>, interval: Duration, path: PathBuf) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        let mut last_saved = state.stats.snapshot();
        loop {
            ticker.tick().await;
            let current = state.stats.snapshot();
            if current == last_saved {
                continue;
            }
            match save(&state.stats, &path).await {
                Ok(()) => last_saved = current,
                Err(e) => warn!("failed to save statistics: {e:#}"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_and_compatibility() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.json");

        // Missing file.
        assert_eq!(load(&path).await.snapshot().total_requests, 0);

        // A file saved by another release.
        std::fs::write(
            &path,
            r#"{"total_requests":5,"cache_hits":2,"cache_misses":3,"last_saved":"x","version":"1.1.1"}"#,
        )
        .unwrap();
        let stats = load(&path).await;
        assert_eq!(stats.snapshot().total_requests, 5);

        stats.record_request();
        save(&stats, &path).await.unwrap();
        assert_eq!(load(&path).await.snapshot().total_requests, 6);

        // Garbage is ignored.
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(load(&path).await.snapshot().total_requests, 0);
    }
}
