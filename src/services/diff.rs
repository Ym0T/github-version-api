//! Version diffs via the GitHub compare API.
//!
//! Both versions must be tags. They are resolved to commits first and the
//! comparison runs between those commits, so a cached diff can never go
//! stale: if a tag is moved, it maps to a different cache key.
//!
//! Diffs to the latest version of an allowed repository are what updaters
//! need; their number is bounded by the number of tags. All other
//! comparisons are charged to the lookup budget, and failures that won't
//! change soon are cached, so clients can't use up the GitHub quota.
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::config::ResolvedRepo;
use crate::error::ApiError;
use crate::models::diff::{CachedDiff, DiffResponse, DiffSummary, FileChange, ZipArtifact};
use crate::services::archive;
use crate::services::github::GitHubError;
use crate::services::repository::ensure_public_repo;
use crate::services::tags;
use crate::services::version::latest_version;
use crate::state::AppState;
use crate::utils::formatting::{format_file_size, rfc3339};

/// The compare API lists at most 300 files and silently drops the rest.
/// Treat a full page as incomplete instead of serving a partial update.
const GITHUB_COMPARE_FILE_LIMIT: usize = 300;

pub async fn get_diff(
    state: &Arc<AppState>,
    target: &ResolvedRepo,
    client: IpAddr,
    from: &str,
    to: &str,
    want_zip: bool,
) -> Result<DiffResponse, ApiError> {
    if want_zip {
        archive::check_zip_allowed(state, target)?;
    }
    ensure_public_repo(state, target).await?;
    let (from_commit, to_commit) = tags::resolve(state, target, from, to).await?;

    // Names, commits and repository keys can't contain ':', so the key is unambiguous.
    let key = format!(
        "{}:{from}:{to}:{from_commit}:{to_commit}",
        target.repo.cache_key()
    );
    if let Some(err) = state.caches.diff_failures.get(&key).await {
        return Err(err);
    }
    let budgeted = !target.trusted || latest_version(state, target).await?.version != to;
    let entry = state
        .caches
        .diffs
        .entry(key.clone())
        .or_try_insert_with(fetch_diff(
            state,
            target,
            budgeted,
            from,
            to,
            &from_commit,
            &to_commit,
        ))
        .await;
    let entry = match entry {
        Ok(entry) => entry,
        Err(err) => {
            let err = Arc::unwrap_or_clone(err);
            if matches!(
                err.code(),
                "diff_too_large" | "version_not_found" | "invalid_version" | "github_inconsistent"
            ) {
                state.caches.diff_failures.insert(key, err.clone()).await;
            }
            return Err(err);
        }
    };
    let fresh = entry.is_fresh();
    if fresh {
        state.stats.record_cache_miss();
        state.diff_store.mark_dirty();
    } else {
        state.stats.record_cache_hit();
    }
    let diff = entry.into_value();

    let mut response = build_response(&diff, !fresh, state.config.diff_cache_ttl);
    if want_zip {
        let artifact = archive::ensure_zip(state, target, client, &key, &diff).await?;
        attach_archive(&mut response, &artifact, state);
    }
    Ok(response)
}

async fn fetch_diff(
    state: &AppState,
    target: &ResolvedRepo,
    budgeted: bool,
    from: &str,
    to: &str,
    from_commit: &str,
    to_commit: &str,
) -> Result<CachedDiff, ApiError> {
    if budgeted {
        state.charge_lookup()?;
    }
    let compare = state
        .github
        .compare(&target.repo, from_commit, to_commit)
        .await
        .map_err(|err| match err {
            GitHubError::NotFound => ApiError::not_found(
                "version_not_found",
                "The versions could not be compared, they may have no common history",
            ),
            GitHubError::Unprocessable => ApiError::unprocessable(
                "invalid_version",
                "GitHub could not compare the requested versions",
            ),
            other => other.into(),
        })?;

    let merge_base_ok = compare.status != "ahead"
        || compare
            .merge_base_commit
            .sha
            .eq_ignore_ascii_case(from_commit);
    if !compare.base_commit.sha.eq_ignore_ascii_case(from_commit) || !merge_base_ok {
        return Err(ApiError::bad_gateway(
            "github_inconsistent",
            "GitHub returned inconsistent data for this comparison",
        ));
    }
    if compare.files.len() >= GITHUB_COMPARE_FILE_LIMIT {
        return Err(ApiError::unprocessable(
            "diff_too_large",
            "The comparison contains 300 or more changed files, which GitHub truncates. Use a full update instead",
        ));
    }

    let files: Vec<FileChange> = compare
        .files
        .into_iter()
        .map(|file| FileChange {
            filename: file.filename,
            status: file.status,
            additions: file.additions,
            deletions: file.deletions,
            changes: file.changes,
            sha: file.sha,
            patch_url: None,
            raw_url: file.raw_url,
            previous_filename: file.previous_filename,
        })
        .collect();
    let now = chrono::Utc::now();

    Ok(CachedDiff {
        repository: target.repo.to_string(),
        from_version: from.to_string(),
        to_version: to.to_string(),
        from_commit: from_commit.to_string(),
        to_commit: to_commit.to_string(),
        comparison_status: compare.status,
        summary: DiffSummary::from_files(&files),
        total_changes: files.iter().map(|f| u64::from(f.changes)).sum(),
        files,
        generated_at: now.to_rfc3339(),
        created_at: now.timestamp(),
        zip: None,
    })
}

fn build_response(diff: &CachedDiff, cached: bool, ttl: Duration) -> DiffResponse {
    let now = chrono::Utc::now().timestamp();
    let remaining = diff.remaining(ttl, now).unwrap_or_default().as_secs();
    DiffResponse {
        from_version: diff.from_version.clone(),
        to_version: diff.to_version.clone(),
        from_commit: diff.from_commit.clone(),
        to_commit: diff.to_commit.clone(),
        comparison_status: diff.comparison_status.clone(),
        repository: diff.repository.clone(),
        total_changes: diff.total_changes,
        files_changed: diff.files.clone(),
        summary: diff.summary.clone(),
        download_url: None,
        zip_size: None,
        zip_size_formatted: None,
        signature: None,
        checksum: None,
        signature_algorithm: None,
        public_key: None,
        manifest: None,
        manifest_signature: None,
        generated_at: diff.generated_at.clone(),
        expires_at: rfc3339(now.saturating_add(i64::try_from(remaining).unwrap_or(i64::MAX))),
        cached,
        cache_expires_in: remaining,
        request_id: uuid::Uuid::new_v4().to_string(),
    }
}

fn attach_archive(response: &mut DiffResponse, artifact: &ZipArtifact, state: &AppState) {
    let base_url = state.config.public_base_url.as_deref().unwrap_or_default();
    let retention = i64::try_from(state.config.zip.retention.as_secs()).unwrap_or(i64::MAX);
    response.download_url = Some(format!("{base_url}/download/diff/{}", artifact.filename));
    response.zip_size = Some(artifact.size);
    response.zip_size_formatted = Some(format_file_size(artifact.size));
    response.checksum = Some(artifact.sha256.clone());
    response.signature.clone_from(&artifact.signature);
    response.signature_algorithm = artifact
        .manifest_signature
        .as_ref()
        .map(|_| "Ed25519".to_string());
    response.public_key.clone_from(&artifact.public_key);
    response.manifest.clone_from(&artifact.manifest);
    response
        .manifest_signature
        .clone_from(&artifact.manifest_signature);
    response.expires_at = rfc3339(artifact.created_at.saturating_add(retention));
}
