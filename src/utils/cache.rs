//! In-memory caches. All caches are bounded in size.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moka::Expiry;
use moka::future::Cache;

use crate::error::ApiError;
use crate::models::diff::CachedDiff;
use crate::models::responses::VersionResponse;

/// How long repository metadata and tag lists are cached.
const LOOKUP_TTL: Duration = Duration::from_secs(300);
/// How long deterministic diff failures (e.g. too many files) are remembered.
const FAILURE_TTL: Duration = Duration::from_secs(600);
/// Upper bounds for the (estimated) memory of the larger caches. The diff
/// limit also bounds the size of the persisted diff cache file.
const MAX_VERSION_CACHE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TAG_CACHE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DIFF_CACHE_BYTES: u64 = 32 * 1024 * 1024;

/// Tag name → commit SHA, and when the list was fetched.
#[derive(Debug)]
pub struct TagList {
    pub commits: HashMap<String, String>,
    pub fetched_at: Instant,
}

pub struct Caches {
    pub versions: Cache<String, CachedVersion>,
    /// `None` = repository missing, not public or not the pinned one.
    pub repos: Cache<String, Option<RepoInfo>>,
    pub tags: Cache<String, Arc<TagList>>,
    pub diffs: Cache<String, CachedDiff>,
    /// Diffs that failed for reasons that won't change soon.
    pub diff_failures: Cache<String, ApiError>,
}

impl Caches {
    /// `zip_reuse_window`: a diff stays cached at least as long as its
    /// archive may be handed out.
    pub fn new(version_ttl: Duration, diff_ttl: Duration, zip_reuse_window: Duration) -> Self {
        Self {
            versions: Cache::builder()
                .weigher(|_key, entry: &CachedVersion| entry.estimated_size())
                .max_capacity(MAX_VERSION_CACHE_BYTES)
                .time_to_live(version_ttl)
                .build(),
            repos: Cache::builder()
                .max_capacity(2_000)
                .time_to_live(LOOKUP_TTL)
                .build(),
            tags: Cache::builder()
                .weigher(|_key, tags: &Arc<TagList>| {
                    let bytes: usize = tags.commits.keys().map(|name| name.len() + 96).sum();
                    u32::try_from(bytes).unwrap_or(u32::MAX)
                })
                .max_capacity(MAX_TAG_CACHE_BYTES)
                .time_to_live(LOOKUP_TTL)
                .build(),
            diffs: Cache::builder()
                .weigher(|_key, diff: &CachedDiff| diff.estimated_size())
                .max_capacity(MAX_DIFF_CACHE_BYTES)
                .expire_after(DiffExpiry {
                    ttl: diff_ttl,
                    zip_reuse_window,
                })
                .build(),
            diff_failures: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(FAILURE_TTL)
                .build(),
        }
    }
}

/// Expires diffs relative to their `created_at` (also for entries loaded
/// from disk), but not while their archive can still be reused.
struct DiffExpiry {
    ttl: Duration,
    zip_reuse_window: Duration,
}

impl DiffExpiry {
    fn remaining(&self, diff: &CachedDiff) -> Duration {
        let now = chrono::Utc::now().timestamp();
        let diff_left = diff.remaining(self.ttl, now).unwrap_or(Duration::ZERO);
        let zip_left = diff
            .zip
            .as_ref()
            .and_then(|zip| {
                let age = u64::try_from(now.checked_sub(zip.created_at)?).ok()?;
                self.zip_reuse_window.checked_sub(Duration::from_secs(age))
            })
            .unwrap_or(Duration::ZERO);
        diff_left.max(zip_left)
    }
}

impl Expiry<String, CachedDiff> for DiffExpiry {
    fn expire_after_create(
        &self,
        _key: &String,
        diff: &CachedDiff,
        _created_at: Instant,
    ) -> Option<Duration> {
        Some(self.remaining(diff))
    }

    fn expire_after_update(
        &self,
        _key: &String,
        diff: &CachedDiff,
        _updated_at: Instant,
        _duration_until_expiry: Option<Duration>,
    ) -> Option<Duration> {
        Some(self.remaining(diff))
    }
}

#[derive(Debug, Clone)]
pub struct CachedVersion {
    response: VersionResponse,
    cached_at: Instant,
}

impl CachedVersion {
    pub fn new(response: VersionResponse) -> Self {
        Self {
            response,
            cached_at: Instant::now(),
        }
    }

    pub fn to_response(&self, ttl: Duration, fresh: bool) -> VersionResponse {
        let mut response = self.response.clone();
        response.cached = !fresh;
        response.cache_expires_in = ttl.saturating_sub(self.cached_at.elapsed()).as_secs();
        if !fresh {
            response.request_id = uuid::Uuid::new_v4().to_string();
        }
        response
    }

    fn estimated_size(&self) -> u32 {
        let r = &self.response;
        let files: usize = r
            .files
            .iter()
            .map(|f| 128 + f.name.len() + f.download_url.len())
            .sum();
        let size = 512
            + r.version.len()
            + r.release_name.as_ref().map_or(0, String::len)
            + r.release_notes.as_ref().map_or(0, String::len)
            + files;
        u32::try_from(size).unwrap_or(u32::MAX)
    }
}

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub default_branch: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::diff::{DiffSummary, ZipArtifact};

    fn diff(created_at: i64, zip_created_at: Option<i64>) -> CachedDiff {
        CachedDiff {
            repository: String::new(),
            from_version: String::new(),
            to_version: String::new(),
            from_commit: String::new(),
            to_commit: String::new(),
            comparison_status: String::new(),
            files: Vec::new(),
            summary: DiffSummary::default(),
            total_changes: 0,
            generated_at: String::new(),
            created_at,
            zip: zip_created_at.map(|created_at| ZipArtifact {
                filename: String::new(),
                size: 0,
                sha256: String::new(),
                created_at,
                signature: None,
                public_key: None,
                manifest: None,
                manifest_signature: None,
            }),
        }
    }

    #[tokio::test]
    async fn diffs_expire_relative_to_creation_but_keep_reusable_archives() {
        let hour = Duration::from_secs(3600);
        let caches = Caches::new(Duration::from_secs(60), hour, 2 * hour);
        let now = chrono::Utc::now().timestamp();
        caches
            .diffs
            .insert("fresh".into(), diff(now - 60, None))
            .await;
        caches
            .diffs
            .insert("old".into(), diff(now - 7200, None))
            .await;
        caches
            .diffs
            .insert("old-with-archive".into(), diff(now - 7200, Some(now - 60)))
            .await;
        caches.diffs.run_pending_tasks().await;
        assert!(caches.diffs.get("fresh").await.is_some());
        assert!(caches.diffs.get("old").await.is_none());
        assert!(caches.diffs.get("old-with-archive").await.is_some());
    }
}
