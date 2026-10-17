//! The fields of GitHub REST API responses that this service reads.
//! Everything else in the payloads is ignored.
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GhRepository {
    pub id: u64,
    pub full_name: String,
    #[serde(default)]
    pub private: bool,
    pub visibility: Option<String>,
    pub default_branch: Option<String>,
}

impl GhRepository {
    pub fn is_public(&self) -> bool {
        !self.private && self.visibility.as_deref().is_none_or(|v| v == "public")
    }
}

#[derive(Debug, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub published_at: Option<String>,
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GhAsset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub download_count: u64,
    pub browser_download_url: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GhTag {
    pub name: String,
    pub commit: GhCommitRef,
}

#[derive(Debug, Deserialize)]
pub struct GhCommitRef {
    pub sha: String,
}

#[derive(Debug, Deserialize)]
pub struct GhCompare {
    /// `ahead`, `behind`, `diverged` or `identical`.
    pub status: String,
    pub base_commit: GhCommitRef,
    pub merge_base_commit: GhCommitRef,
    #[serde(default)]
    pub files: Vec<GhFile>,
}

#[derive(Debug, Deserialize)]
pub struct GhFile {
    pub filename: String,
    pub status: String,
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
    #[serde(default)]
    pub changes: u32,
    pub sha: Option<String>,
    pub raw_url: Option<String>,
    pub previous_filename: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GhTree {
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub tree: Vec<GhTreeEntry>,
}

#[derive(Debug, Deserialize)]
pub struct GhTreeEntry {
    pub path: String,
    pub mode: String,
    pub sha: String,
    /// Blob size in bytes (absent for trees and submodules).
    pub size: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GhPackageJson {
    pub name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
}
