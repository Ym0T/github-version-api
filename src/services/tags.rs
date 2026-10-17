//! Tags of a repository, resolved to commit SHAs.
//!
//! Diffs are only computed between tags, and always via the commits the tags
//! point to. Unknown version names are rejected from the cached tag list
//! without any GitHub request, so made-up versions can't use up the quota.
//! A list that misses a requested tag is refreshed, at most every
//! [`REFRESH_AFTER`], so a new release is found quickly.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::ResolvedRepo;
use crate::error::ApiError;
use crate::state::AppState;
use crate::utils::cache::TagList;
use crate::validation::{is_sha1_hex, is_valid_version};

/// Up to 1000 tags are considered for allowed repositories, 100 otherwise.
const TRUSTED_PAGES: u32 = 10;
const UNTRUSTED_PAGES: u32 = 1;
const REFRESH_AFTER: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_secs(30)
};

/// Commits of `from` and `to`.
pub async fn resolve(
    state: &AppState,
    target: &ResolvedRepo,
    from: &str,
    to: &str,
) -> Result<(String, String), ApiError> {
    let mut tags = tag_list(state, target).await?;
    let known = |tags: &TagList| tags.commits.contains_key(from) && tags.commits.contains_key(to);
    // Early refreshes are charged to the lookup budget, so made-up versions
    // can't trigger them in a loop; without budget the list just expires.
    if !known(&tags) && tags.fetched_at.elapsed() >= REFRESH_AFTER && state.charge_lookup().is_ok()
    {
        state.caches.tags.invalidate(&target.repo.cache_key()).await;
        tags = tag_list(state, target).await?;
    }
    Ok((commit_of(&tags, from)?, commit_of(&tags, to)?))
}

async fn tag_list(state: &AppState, target: &ResolvedRepo) -> Result<Arc<TagList>, ApiError> {
    state
        .caches
        .tags
        .entry(target.repo.cache_key())
        .or_try_insert_with(async {
            let pages = if target.trusted {
                TRUSTED_PAGES
            } else {
                state.charge_lookup()?;
                UNTRUSTED_PAGES
            };
            let tags = state.github.tags(&target.repo, pages).await?;
            let commits: HashMap<String, String> = tags
                .into_iter()
                .filter(|tag| is_valid_version(&tag.name) && is_sha1_hex(&tag.commit.sha))
                .map(|tag| (tag.name, tag.commit.sha.to_ascii_lowercase()))
                .collect();
            Ok::<_, ApiError>(Arc::new(TagList {
                commits,
                fetched_at: Instant::now(),
            }))
        })
        .await
        .map(moka::Entry::into_value)
        .map_err(Arc::unwrap_or_clone)
}

fn commit_of(tags: &TagList, version: &str) -> Result<String, ApiError> {
    tags.commits.get(version).cloned().ok_or_else(|| {
        ApiError::not_found(
            "version_not_found",
            "Diffs are only available between existing tags of the repository",
        )
    })
}
