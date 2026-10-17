//! Diff models: API response and the cached/persisted representation.
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct DiffResponse {
    pub from_version: String,
    pub to_version: String,
    /// Commits the version tags pointed to when the diff was computed.
    pub from_commit: String,
    pub to_commit: String,
    /// GitHub comparison status: `ahead`, `behind`, `diverged` or `identical`.
    /// Only `ahead` describes a complete update from `from_version` to `to_version`.
    pub comparison_status: String,
    pub repository: String,
    pub total_changes: u64,
    pub files_changed: Vec<FileChange>,
    pub summary: DiffSummary,
    pub download_url: Option<String>,
    pub zip_size: Option<u64>,
    pub zip_size_formatted: Option<String>,
    /// Legacy (opt-in): base64 Ed25519 signature over the raw 32-byte SHA-256 of the ZIP.
    pub signature: Option<String>,
    /// Hex SHA-256 of the ZIP.
    pub checksum: Option<String>,
    pub signature_algorithm: Option<String>,
    /// Informational only. Clients must verify with their own pinned key.
    pub public_key: Option<String>,
    /// Signed update manifest that binds the ZIP to repository, versions,
    /// commits and the files to delete.
    pub manifest: Option<String>,
    /// Base64 Ed25519 signature over the exact bytes of `manifest`.
    pub manifest_signature: Option<String>,
    pub generated_at: String,
    pub expires_at: String,
    pub cached: bool,
    pub cache_expires_in: u64,
    pub request_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct DiffSummary {
    pub total_additions: u64,
    pub total_deletions: u64,
    pub files_added: u32,
    pub files_modified: u32,
    pub files_removed: u32,
    pub files_renamed: u32,
}

impl DiffSummary {
    pub fn from_files(files: &[FileChange]) -> Self {
        let mut summary = Self::default();
        for file in files {
            summary.total_additions += u64::from(file.additions);
            summary.total_deletions += u64::from(file.deletions);
            match file.status.as_str() {
                "added" => summary.files_added += 1,
                "modified" => summary.files_modified += 1,
                "removed" => summary.files_removed += 1,
                "renamed" => summary.files_renamed += 1,
                _ => {}
            }
        }
        summary
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub filename: String,
    /// `added`, `modified`, `removed`, `renamed`, `copied`, `changed` or `unchanged`.
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub changes: u32,
    /// Git blob SHA of the file in `to_version`.
    pub sha: Option<String>,
    /// Always `null`; kept for response compatibility.
    pub patch_url: Option<String>,
    pub raw_url: Option<String>,
    pub previous_filename: Option<String>,
}

impl FileChange {
    pub fn is_removed(&self) -> bool {
        self.status == "removed"
    }
}

/// A diff as stored in memory and in the persistent diff cache. Diffs are
/// computed between commit SHAs, so an entry never becomes stale; it only
/// expires to bound memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDiff {
    pub repository: String,
    pub from_version: String,
    pub to_version: String,
    pub from_commit: String,
    pub to_commit: String,
    pub comparison_status: String,
    pub files: Vec<FileChange>,
    pub summary: DiffSummary,
    pub total_changes: u64,
    pub generated_at: String,
    /// Unix timestamp.
    pub created_at: i64,
    pub zip: Option<ZipArtifact>,
}

impl CachedDiff {
    /// Remaining lifetime for a cache TTL. `None` if already expired or
    /// created "in the future" (clock changes, edited files).
    pub fn remaining(&self, ttl: Duration, now: i64) -> Option<Duration> {
        if self.created_at > now {
            return None;
        }
        let age = Duration::from_secs(now.abs_diff(self.created_at));
        ttl.checked_sub(age).filter(|left| !left.is_zero())
    }

    /// Rough serialized size in bytes, used as cache weight.
    pub fn estimated_size(&self) -> u32 {
        let files: usize = self
            .files
            .iter()
            .map(|f| {
                128 + f.filename.len()
                    + f.status.len()
                    + f.raw_url.as_ref().map_or(0, String::len)
                    + f.previous_filename.as_ref().map_or(0, String::len)
            })
            .sum();
        let zip = self
            .zip
            .as_ref()
            .map_or(0, |z| 512 + z.manifest.as_ref().map_or(0, String::len));
        u32::try_from(512 + files + zip).unwrap_or(u32::MAX)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZipArtifact {
    pub filename: String,
    pub size: u64,
    /// Hex SHA-256 of the archive.
    pub sha256: String,
    /// Unix timestamp.
    pub created_at: i64,
    /// Legacy signature, only present if legacy signatures are enabled.
    pub signature: Option<String>,
    pub public_key: Option<String>,
    pub manifest: Option<String>,
    pub manifest_signature: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(status: &str, additions: u32, deletions: u32) -> FileChange {
        FileChange {
            filename: "f".into(),
            status: status.into(),
            additions,
            deletions,
            changes: additions + deletions,
            sha: None,
            patch_url: None,
            raw_url: None,
            previous_filename: None,
        }
    }

    #[test]
    fn summary_counts_without_overflow() {
        let files = vec![
            change("added", u32::MAX, 0),
            change("added", u32::MAX, 0),
            change("modified", 1, 2),
            change("removed", 0, 5),
            change("renamed", 0, 0),
        ];
        let summary = DiffSummary::from_files(&files);
        assert_eq!(summary.total_additions, 2 * u64::from(u32::MAX) + 1);
        assert_eq!(summary.total_deletions, 7);
        assert_eq!(
            (
                summary.files_added,
                summary.files_modified,
                summary.files_removed,
                summary.files_renamed
            ),
            (2, 1, 1, 1)
        );
    }

    #[test]
    fn remaining_lifetime() {
        let diff = |created_at| CachedDiff {
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
            zip: None,
        };
        let hour = Duration::from_secs(3600);
        let now = 1_000_000;
        assert_eq!(
            diff(now - 60).remaining(hour, now),
            Some(Duration::from_secs(3540))
        );
        assert_eq!(diff(now - 3600).remaining(hour, now), None);
        assert_eq!(diff(now + 10).remaining(hour, now), None);
    }
}
