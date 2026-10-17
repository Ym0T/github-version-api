//! Response bodies for the version, health and error endpoints.
use serde::Serialize;

use crate::utils::formatting::current_timestamp;

#[derive(Debug, Serialize, Clone)]
pub struct VersionResponse {
    pub version: String,
    pub release_name: Option<String>,
    pub published_at: String,
    pub files: Vec<FileInfo>,
    pub download_urls: DownloadUrls,
    pub release_notes: Option<String>,
    pub cached: bool,
    /// Seconds until the cached value expires.
    pub cache_expires_in: u64,
    pub request_id: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub size_formatted: String,
    pub download_count: u64,
    pub download_url: String,
    pub content_type: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DownloadUrls {
    pub zipball: String,
    pub tarball: String,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub uptime: String,
    pub timestamp: String,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
    pub request_id: String,
    pub timestamp: String,
}

impl ErrorResponse {
    pub fn new(code: &str, message: String) -> Self {
        Self {
            error: code.to_string(),
            message,
            request_id: uuid::Uuid::new_v4().to_string(),
            timestamp: current_timestamp(),
        }
    }
}
