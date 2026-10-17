//! Repository checks done before any data is served.
//!
//! - The repository must be public, even if the token could see private ones.
//! - GitHub must report the same name that was requested; a renamed or
//!   transferred repository is reached through a redirect and is refused.
//! - An id pinned in `ALLOWED_REPOS` (`owner/repo#id`) must match. Only the
//!   pin detects a different repository registered under the old name later
//!   ("repojacking"), which is why archives require it.

use std::sync::Arc;

use tracing::{info, warn};

use crate::config::ResolvedRepo;
use crate::error::ApiError;
use crate::services::github::GitHubError;
use crate::state::AppState;
use crate::utils::cache::RepoInfo;

pub async fn ensure_public_repo(
    state: &AppState,
    target: &ResolvedRepo,
) -> Result<RepoInfo, ApiError> {
    let repo = &target.repo;
    let info = state
        .caches
        .repos
        .entry(repo.cache_key())
        .or_try_insert_with(async {
            if !target.trusted {
                state.charge_lookup()?;
            }
            let details = match state.github.repository(repo).await {
                Ok(details) => details,
                Err(GitHubError::NotFound) => return Ok(None),
                Err(err) => return Err(ApiError::from(err)),
            };
            if !details.is_public() {
                return Ok(None);
            }
            if !details.full_name.eq_ignore_ascii_case(&repo.to_string()) {
                warn!(requested = %repo, actual = %details.full_name, "repository was renamed or transferred, refusing");
                return Ok(None);
            }
            match target.pinned_id {
                Some(pinned) if pinned != details.id => {
                    warn!(repository = %repo, pinned, actual = details.id, "repository id does not match the pinned id, refusing");
                    return Ok(None);
                }
                None if target.trusted => {
                    info!(repository = %repo, id = details.id, "pin this id in ALLOWED_REPOS as {repo}#{}", details.id);
                }
                _ => {}
            }
            Ok(Some(RepoInfo {
                default_branch: details.default_branch.filter(|b| is_plain_branch_name(b)),
            }))
        })
        .await
        .map_err(Arc::unwrap_or_clone)?
        .into_value();

    info.ok_or_else(|| {
        ApiError::not_found("repository_not_found", "Repository not found or not public")
    })
}

/// Branch names used to build raw file URLs.
fn is_plain_branch_name(branch: &str) -> bool {
    (1..=255).contains(&branch.len())
        && branch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
        && !branch.contains("..")
        && !branch.contains("//")
        && !branch.starts_with('/')
        && !branch.ends_with('/')
}

#[cfg(test)]
mod tests {
    use super::is_plain_branch_name;

    #[test]
    fn branch_names() {
        assert!(is_plain_branch_name("main"));
        assert!(is_plain_branch_name("release/2.x"));
        assert!(!is_plain_branch_name("../x"));
        assert!(!is_plain_branch_name("a//b"));
        assert!(!is_plain_branch_name("a?b"));
        assert!(!is_plain_branch_name(""));
    }
}
