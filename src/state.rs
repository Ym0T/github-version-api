//! State shared by all request handlers.
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;

use crate::config::AppConfig;
use crate::error::ApiError;
use crate::services::archive::ZipBuildGuard;
use crate::services::github::GitHubClient;
use crate::services::signing::Signer;
use crate::utils::budget::HourlyBudget;
use crate::utils::cache::Caches;
use crate::utils::persistent_diff_cache::DiffCacheStore;
use crate::utils::stats::Stats;

/// Context for the key that authenticates the persisted diff cache.
const DIFF_CACHE_MAC_CONTEXT: &str = "version-api/diff-cache-mac/v1";

pub struct AppState {
    pub config: AppConfig,
    pub github: GitHubClient,
    pub signer: Option<Signer>,
    pub caches: Caches,
    pub diff_store: DiffCacheStore,
    pub zip_guard: ZipBuildGuard,
    /// GitHub lookups that updates to the latest version of an allowed
    /// repository don't need (see `LOOKUP_BUDGET_PER_HOUR`).
    lookup_budget: HourlyBudget,
    pub stats: Stats,
    pub started_at: Instant,
}

impl AppState {
    pub async fn new(config: AppConfig, signer: Option<Signer>) -> anyhow::Result<Arc<Self>> {
        let github = GitHubClient::new(config.http_timeout, config.github_token.as_ref())?;
        Self::with_github(config, signer, github).await
    }

    pub async fn with_github(
        config: AppConfig,
        signer: Option<Signer>,
        github: GitHubClient,
    ) -> anyhow::Result<Arc<Self>> {
        crate::utils::fs::create_private_dir_all(&config.diffs_dir)
            .await
            .with_context(|| format!("cannot create {}", config.diffs_dir.display()))?;

        let caches = Caches::new(
            config.version_cache_ttl,
            config.diff_cache_ttl,
            config.zip.reuse_window(),
        );
        let stats = crate::utils::persistent_stats::load(&config.stats_file).await;
        let mac_key = signer
            .as_ref()
            .map(|signer| signer.derive_key(DIFF_CACHE_MAC_CONTEXT));
        let diff_store = DiffCacheStore::new(config.diff_cache_file.clone(), mac_key);
        diff_store.load_into(&caches.diffs).await;

        Ok(Arc::new(Self {
            zip_guard: ZipBuildGuard::new(&config.zip),
            lookup_budget: HourlyBudget::new(config.lookup_budget_per_hour),
            config,
            github,
            signer,
            caches,
            diff_store,
            stats,
            started_at: Instant::now(),
        }))
    }

    /// Charges one budgeted GitHub lookup.
    pub fn charge_lookup(&self) -> Result<(), ApiError> {
        self.lookup_budget.try_take().map_err(|retry_after| {
            ApiError::unavailable(
                "lookup_limit_reached",
                "The hourly limit for lookups beyond updates to the latest version is reached, please retry later",
                Some(retry_after),
            )
        })
    }

    /// Writes stats and the diff cache to disk.
    pub async fn persist(&self) {
        if let Err(e) =
            crate::utils::persistent_stats::save(&self.stats, &self.config.stats_file).await
        {
            tracing::warn!("failed to save statistics: {e:#}");
        }
        self.diff_store.flush(&self.caches.diffs).await;
    }
}
