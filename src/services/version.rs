//! Latest-version lookup.
//!
//! Sources, in order: latest release, newest tag, `VERSION` file and
//! `package.json` on the default branch. Only "not found" (or a version name
//! that fails validation) falls through to the next source; other GitHub
//! errors are returned so that an outage is never cached as "no version".
//! Version names are validated because update scripts interpolate them.

use std::sync::Arc;

use crate::config::{RepoRef, ResolvedRepo};
use crate::error::ApiError;
use crate::models::github::{GhPackageJson, GhRelease};
use crate::models::responses::{DownloadUrls, FileInfo, VersionResponse};
use crate::services::github::GitHubClient;
use crate::services::repository::ensure_public_repo;
use crate::state::AppState;
use crate::utils::cache::{CachedVersion, RepoInfo};
use crate::utils::formatting::{current_timestamp, format_file_size};
use crate::validation::{clean_text, clean_version_string, is_valid_version};

const MAX_ASSETS: usize = 100;
const MAX_RELEASE_NOTES_CHARS: usize = 100_000;
const MAX_NAME_CHARS: usize = 200;

pub async fn latest_version(
    state: &AppState,
    target: &ResolvedRepo,
) -> Result<VersionResponse, ApiError> {
    let entry = state
        .caches
        .versions
        .entry(target.repo.cache_key())
        .or_try_insert_with(async {
            let info = ensure_public_repo(state, target).await?;
            if !target.trusted {
                state.charge_lookup()?;
            }
            let response = lookup(&state.github, &target.repo, &info).await?;
            Ok::<_, ApiError>(CachedVersion::new(response))
        })
        .await
        .map_err(Arc::unwrap_or_clone)?;

    let fresh = entry.is_fresh();
    if fresh {
        state.stats.record_cache_miss();
    } else {
        state.stats.record_cache_hit();
    }
    Ok(entry
        .into_value()
        .to_response(state.config.version_cache_ttl, fresh))
}

async fn lookup(
    github: &GitHubClient,
    repo: &RepoRef,
    info: &RepoInfo,
) -> Result<VersionResponse, ApiError> {
    if let Some(release) = github.latest_release(repo).await?
        && is_valid_version(&release.tag_name)
    {
        return Ok(from_release(github, repo, release));
    }
    if let Some(tag) = github
        .tags(repo, 1)
        .await?
        .into_iter()
        .find(|tag| is_valid_version(&tag.name))
    {
        return Ok(VersionResponse {
            release_name: Some(format!("{repo} - {}", tag.name)),
            release_notes: Some("Retrieved from GitHub Tags API".into()),
            download_urls: archive_urls(github, repo, Some(&tag.name)),
            ..base_response(tag.name)
        });
    }

    if let Some(branch) = &info.default_branch {
        if let Some(text) = github.raw_text(repo, branch, "VERSION").await?
            && let Some(version) = clean_version_string(&text)
        {
            return Ok(VersionResponse {
                release_name: Some(repo.to_string()),
                release_notes: Some("From VERSION file".into()),
                download_urls: archive_urls(github, repo, None),
                ..base_response(version)
            });
        }
        if let Some(text) = github.raw_text(repo, branch, "package.json").await?
            && let Ok(package) = serde_json::from_str::<GhPackageJson>(&text)
            && let Some(version) = package.version.as_deref().and_then(clean_version_string)
        {
            let name = clean_text(
                package.name.as_deref().unwrap_or(&repo.name),
                MAX_NAME_CHARS,
            );
            let description =
                clean_text(package.description.as_deref().unwrap_or(""), MAX_NAME_CHARS);
            return Ok(VersionResponse {
                release_name: Some(repo.to_string()),
                release_notes: Some(format!("{name}: {description}")),
                download_urls: archive_urls(github, repo, None),
                ..base_response(version)
            });
        }
    }

    Ok(VersionResponse {
        release_name: Some(repo.to_string()),
        release_notes: Some("Repository accessible but no version information available".into()),
        download_urls: archive_urls(github, repo, None),
        ..base_response("latest".into())
    })
}

fn from_release(github: &GitHubClient, repo: &RepoRef, release: GhRelease) -> VersionResponse {
    let files = release
        .assets
        .into_iter()
        .take(MAX_ASSETS)
        .map(|asset| FileInfo {
            name: clean_text(&asset.name, MAX_NAME_CHARS),
            size: asset.size,
            size_formatted: format_file_size(asset.size),
            download_count: asset.download_count,
            download_url: asset.browser_download_url,
            content_type: asset.content_type.map(|t| clean_text(&t, MAX_NAME_CHARS)),
        })
        .collect();

    VersionResponse {
        release_name: release.name.map(|n| clean_text(&n, MAX_NAME_CHARS)),
        published_at: release.published_at.unwrap_or_else(current_timestamp),
        files,
        download_urls: archive_urls(github, repo, Some(&release.tag_name)),
        release_notes: Some(
            release
                .body
                .unwrap_or_default()
                .chars()
                .take(MAX_RELEASE_NOTES_CHARS)
                .collect(),
        ),
        ..base_response(release.tag_name)
    }
}

fn base_response(version: String) -> VersionResponse {
    VersionResponse {
        version,
        release_name: None,
        published_at: current_timestamp(),
        files: Vec::new(),
        download_urls: DownloadUrls {
            zipball: String::new(),
            tarball: String::new(),
        },
        release_notes: None,
        cached: false,
        cache_expires_in: 0,
        request_id: uuid::Uuid::new_v4().to_string(),
    }
}

/// zipball/tarball URLs for a ref, or the default branch if `git_ref` is `None`.
fn archive_urls(github: &GitHubClient, repo: &RepoRef, git_ref: Option<&str>) -> DownloadUrls {
    let url = |kind: &str| {
        let mut segments = vec!["repos", repo.owner.as_str(), repo.name.as_str(), kind];
        segments.extend(git_ref);
        github.api_url(&segments).to_string()
    };
    DownloadUrls {
        zipball: url("zipball"),
        tarball: url("tarball"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn archive_urls_are_encoded() {
        let github = GitHubClient::new(Duration::from_secs(5), None).unwrap();
        let repo: RepoRef = "o/r".parse().unwrap();
        let urls = archive_urls(&github, &repo, Some("v1.0/../../x?y"));
        assert_eq!(
            urls.zipball,
            "https://api.github.com/repos/o/r/zipball/v1.0%2F..%2F..%2Fx%3Fy"
        );
        let urls = archive_urls(&github, &repo, None);
        assert_eq!(urls.tarball, "https://api.github.com/repos/o/r/tarball");
    }
}
